"use client";

import { useEffect, useState, useCallback } from "react";
import Nav from "../../components/nav";
import Link from "next/link";

interface Artifact {
  type: string;
  id: string;
  title: string;
  status: string;
  metadata: Record<string, string>;
  content: string;
}

interface StatusResponse {
  mission: {
    id: string;
    phase: string;
    objective: string;
  };
  kb: {
    artifacts: Artifact[];
    phase: string;
    artifactCounts: {
      hypotheses: number;
      experiments: number;
      findings: number;
      challengeReviews: number;
      tasks: number;
    };
  };
  challenge?: {
    successMetric: string;
  };
}

const IMPACT_COLORS: Record<string, string> = {
  HIGH: "#198038",
  MEDIUM: "#f1c21b",
  LOW: "#525252",
};

const IMPACT_TEXT_COLORS: Record<string, string> = {
  HIGH: "#ffffff",
  MEDIUM: "#161616",
  LOW: "#ffffff",
};

const IMPACT_ORDER: Record<string, number> = {
  HIGH: 0,
  MEDIUM: 1,
  LOW: 2,
};

function impactBadge(impact: string) {
  const key = impact.toUpperCase();
  return (
    <span
      style={{
        display: "inline-block",
        fontSize: "11px",
        fontWeight: 500,
        fontFamily: "'IBM Plex Sans', sans-serif",
        color: IMPACT_TEXT_COLORS[key] || "#ffffff",
        backgroundColor: IMPACT_COLORS[key] || "#525252",
        padding: "1px 8px",
        borderRadius: "2px",
        lineHeight: "18px",
      }}
    >
      {impact}
    </span>
  );
}

/**
 * Parse the Parameters table from experiment markdown content.
 * Looks for a markdown table under a "Parameters" heading.
 */
function parseParametersTable(
  content: string
): Array<{ key: string; value: string }> {
  const params: Array<{ key: string; value: string }> = [];
  const lines = content.split("\n");

  let inParams = false;
  for (const line of lines) {
    if (/^##?\s+parameters/i.test(line.trim())) {
      inParams = true;
      continue;
    }
    if (inParams && /^##?\s+/i.test(line.trim()) && !/parameters/i.test(line)) {
      break;
    }
    if (inParams) {
      const match = line.match(/\|\s*(.+?)\s*\|\s*(.+?)\s*\|/);
      if (match && !match[1].includes("---") && !match[1].toLowerCase().includes("parameter")) {
        params.push({ key: match[1].trim(), value: match[2].trim() });
      }
    }
  }
  return params;
}

/**
 * Parse the Results section from experiment markdown content.
 */
function parseResultsSection(content: string): string {
  const lines = content.split("\n");
  let inResults = false;
  const resultLines: string[] = [];

  for (const line of lines) {
    if (/^##?\s+results/i.test(line.trim())) {
      inResults = true;
      continue;
    }
    if (inResults && /^##?\s+/i.test(line.trim()) && !/results/i.test(line)) {
      break;
    }
    if (inResults) {
      resultLines.push(line);
    }
  }
  return resultLines.join("\n").trim();
}

/**
 * Extract evidence summary from finding content.
 * Takes the first paragraph after metadata blockquotes.
 */
function extractEvidenceSummary(content: string): string {
  const lines = content.split("\n");
  const summaryLines: string[] = [];
  let pastMeta = false;
  let pastTitle = false;

  for (const line of lines) {
    if (line.startsWith("#")) {
      pastTitle = true;
      continue;
    }
    if (!pastTitle) continue;
    if (line.startsWith(">")) {
      pastMeta = true;
      continue;
    }
    if (pastMeta && line.trim() === "") {
      if (summaryLines.length > 0) break;
      continue;
    }
    if (pastMeta && line.trim()) {
      summaryLines.push(line.trim());
    }
  }
  const summary = summaryLines.join(" ");
  return summary.length > 300 ? summary.slice(0, 297) + "..." : summary;
}

/**
 * Calculate max bar value for chart scaling.
 */
function getBarChartData(experiments: Artifact[]): Array<{
  id: string;
  title: string;
  value: number;
  label: string;
}> {
  return experiments.map((exp) => {
    // Try to extract a numeric result from metadata or results section
    const resultValue =
      exp.metadata["result"] ||
      exp.metadata["score"] ||
      exp.metadata["improvement"] ||
      "";
    const numMatch = resultValue.match(/[\d.]+/);
    const value = numMatch ? parseFloat(numMatch[0]) : 0;
    return {
      id: exp.id,
      title: exp.title,
      value,
      label: resultValue || "N/A",
    };
  });
}

export default function FindingsPage() {
  const [data, setData] = useState<StatusResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [selectedExperiment, setSelectedExperiment] = useState<string>("");

  const fetchStatus = useCallback(async () => {
    try {
      const res = await fetch("/api/status");
      if (!res.ok) return;
      const json: StatusResponse = await res.json();
      setData(json);
    } catch {
      // silent — retry on next poll
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchStatus();
    const interval = setInterval(fetchStatus, 5000);
    return () => clearInterval(interval);
  }, [fetchStatus]);

  const findings = data
    ? data.kb.artifacts
        .filter((a) => a.type === "finding")
        .sort((a, b) => {
          const aImpact = (a.metadata.impact || a.metadata["impact level"] || "LOW").toUpperCase();
          const bImpact = (b.metadata.impact || b.metadata["impact level"] || "LOW").toUpperCase();
          const orderDiff = (IMPACT_ORDER[aImpact] ?? 3) - (IMPACT_ORDER[bImpact] ?? 3);
          if (orderDiff !== 0) return orderDiff;
          return a.id.localeCompare(b.id);
        })
    : [];

  const experiments = data
    ? data.kb.artifacts.filter((a) => a.type === "experiment")
    : [];

  const barData = getBarChartData(experiments);
  const maxBarValue = Math.max(...barData.map((d) => d.value), 1);

  const selectedExp = experiments.find((e) => e.id === selectedExperiment);

  const isEmpty = !data || (findings.length === 0 && experiments.length === 0);

  // Extract success metric from challenge data or from mission objective
  const successMetric = data?.challenge?.successMetric || "";

  return (
    <div style={{ minHeight: "100vh", marginLeft: 64, backgroundColor: "#f4f4f4", fontFamily: "'IBM Plex Sans', sans-serif" }}>
      <Nav activePath="/findings" />

      <div style={{ maxWidth: "960px", margin: "0 auto", padding: "24px" }}>
        <h1
          style={{
            fontSize: "20px",
            fontWeight: 600,
            color: "#161616",
            marginBottom: "8px",
          }}
        >
          Findings &amp; Metrics
        </h1>
        <p style={{ fontSize: "14px", color: "#525252", marginBottom: "24px" }}>
          Key findings ranked by impact with experiment data
        </p>

        {loading && (
          <div
            style={{
              backgroundColor: "#ffffff",
              border: "1px solid #e0e0e0",
              padding: "32px",
              textAlign: "center",
              fontSize: "14px",
              color: "#525252",
            }}
          >
            Loading findings data...
          </div>
        )}

        {!loading && isEmpty && (
          <div
            style={{
              backgroundColor: "#ffffff",
              border: "1px solid #e0e0e0",
              padding: "48px 32px",
              textAlign: "center",
              fontSize: "14px",
              color: "#525252",
            }}
          >
            No findings yet. The agent is still testing hypotheses.
          </div>
        )}

        {!loading && !isEmpty && (
          <>
            {/* Success Metric */}
            {successMetric && (
              <div
                style={{
                  backgroundColor: "#ffffff",
                  border: "1px solid #e0e0e0",
                  padding: "16px",
                  marginBottom: "16px",
                }}
              >
                <div
                  style={{
                    fontSize: "11px",
                    fontWeight: 600,
                    color: "#525252",
                    textTransform: "uppercase",
                    letterSpacing: "0.5px",
                    marginBottom: "8px",
                  }}
                >
                  Success Metric
                </div>
                <div style={{ fontSize: "14px", color: "#161616" }}>
                  {successMetric}
                </div>
              </div>
            )}

            {/* Bar Chart — Results by Approach */}
            {barData.length > 0 && barData.some((d) => d.value > 0) && (
              <div
                style={{
                  backgroundColor: "#ffffff",
                  border: "1px solid #e0e0e0",
                  padding: "16px",
                  marginBottom: "16px",
                }}
              >
                <div
                  style={{
                    fontSize: "11px",
                    fontWeight: 600,
                    color: "#525252",
                    textTransform: "uppercase",
                    letterSpacing: "0.5px",
                    marginBottom: "16px",
                  }}
                >
                  Results by Approach
                </div>
                <div style={{ display: "flex", flexDirection: "column", gap: "8px" }}>
                  {barData.map((bar) => (
                    <div
                      key={bar.id}
                      style={{
                        display: "flex",
                        alignItems: "center",
                        gap: "8px",
                      }}
                    >
                      <span
                        style={{
                          fontFamily: "'IBM Plex Mono', monospace",
                          fontSize: "12px",
                          color: "#525252",
                          width: "48px",
                          flexShrink: 0,
                        }}
                      >
                        {bar.id}
                      </span>
                      <div
                        style={{
                          flex: 1,
                          height: "24px",
                          backgroundColor: "#e0e0e0",
                          position: "relative",
                        }}
                      >
                        <div
                          style={{
                            height: "100%",
                            width: `${Math.max((bar.value / maxBarValue) * 100, 2)}%`,
                            backgroundColor: "#0f62fe",
                            transition: "width 0.3s ease",
                          }}
                        />
                      </div>
                      <span
                        style={{
                          fontFamily: "'IBM Plex Mono', monospace",
                          fontSize: "12px",
                          color: "#161616",
                          width: "80px",
                          textAlign: "right",
                          flexShrink: 0,
                        }}
                      >
                        {bar.label}
                      </span>
                    </div>
                  ))}
                </div>
              </div>
            )}

            {/* Key Findings */}
            {findings.length > 0 && (
              <div style={{ marginBottom: "16px" }}>
                <div
                  style={{
                    fontSize: "11px",
                    fontWeight: 600,
                    color: "#525252",
                    textTransform: "uppercase",
                    letterSpacing: "0.5px",
                    marginBottom: "8px",
                  }}
                >
                  Key Findings
                </div>
                <div style={{ display: "flex", flexDirection: "column", gap: "4px" }}>
                  {findings.map((f) => {
                    const impact = (f.metadata.impact || f.metadata["impact level"] || "").toUpperCase();
                    const linkedExp = f.metadata.experiment || "";
                    const evidenceSummary = extractEvidenceSummary(f.content);

                    return (
                      <div
                        key={f.id}
                        style={{
                          backgroundColor: "#ffffff",
                          border: "1px solid #e0e0e0",
                          borderLeft: "3px solid #198038",
                          padding: "12px 16px",
                        }}
                      >
                        <div
                          style={{
                            display: "flex",
                            alignItems: "center",
                            gap: "8px",
                            marginBottom: "8px",
                          }}
                        >
                          <span
                            style={{
                              fontFamily: "'IBM Plex Mono', monospace",
                              fontSize: "12px",
                              fontWeight: 500,
                              color: "#198038",
                            }}
                          >
                            <Link href={`/artifact/${f.id}`} style={{color: "inherit", textDecoration: "none"}}>{f.id}</Link>
                          </span>
                          <span style={{ fontSize: "14px", color: "#161616", flex: 1 }}>
                            <Link href={`/artifact/${f.id}`} style={{color: "#161616", textDecoration: "none"}}>{f.title}</Link>
                          </span>
                          {impact && impactBadge(impact)}
                        </div>
                        {evidenceSummary && (
                          <p
                            style={{
                              fontSize: "13px",
                              color: "#525252",
                              lineHeight: "20px",
                              marginBottom: "8px",
                            }}
                          >
                            {evidenceSummary}
                          </p>
                        )}
                        {linkedExp && (
                          <span
                            style={{
                              fontSize: "11px",
                              fontFamily: "'IBM Plex Mono', monospace",
                              color: "#a8a8a8",
                            }}
                          >
                            Experiment: {linkedExp}
                          </span>
                        )}
                      </div>
                    );
                  })}
                </div>
              </div>
            )}

            {/* Experiment Data */}
            {experiments.length > 0 && (
              <div style={{ marginBottom: "16px" }}>
                <div
                  style={{
                    fontSize: "11px",
                    fontWeight: 600,
                    color: "#525252",
                    textTransform: "uppercase",
                    letterSpacing: "0.5px",
                    marginBottom: "8px",
                  }}
                >
                  Experiment Data
                </div>
                <div
                  style={{
                    backgroundColor: "#ffffff",
                    border: "1px solid #e0e0e0",
                    padding: "16px",
                  }}
                >
                  <div style={{ marginBottom: "16px" }}>
                    <label
                      htmlFor="experiment-select"
                      style={{
                        fontSize: "13px",
                        color: "#525252",
                        display: "block",
                        marginBottom: "4px",
                      }}
                    >
                      Select experiment
                    </label>
                    <select
                      id="experiment-select"
                      value={selectedExperiment}
                      onChange={(e) => setSelectedExperiment(e.target.value)}
                      style={{
                        fontFamily: "'IBM Plex Sans', sans-serif",
                        fontSize: "14px",
                        padding: "8px 12px",
                        border: "1px solid #e0e0e0",
                        borderRadius: "0px",
                        backgroundColor: "#ffffff",
                        color: "#161616",
                        width: "100%",
                        appearance: "auto",
                      }}
                    >
                      <option value="">Choose an experiment...</option>
                      {experiments.map((exp) => (
                        <option key={exp.id} value={exp.id}>
                          {exp.id} — {exp.title}
                        </option>
                      ))}
                    </select>
                  </div>

                  {selectedExp && (
                    <>
                      {/* Parameters Table */}
                      {(() => {
                        const params = parseParametersTable(selectedExp.content);
                        if (params.length === 0) return null;
                        return (
                          <div style={{ marginBottom: "16px" }}>
                            <div
                              style={{
                                fontSize: "12px",
                                fontWeight: 600,
                                color: "#161616",
                                marginBottom: "8px",
                              }}
                            >
                              Parameters
                            </div>
                            <table
                              style={{
                                width: "100%",
                                borderCollapse: "collapse",
                                fontSize: "13px",
                              }}
                            >
                              <thead>
                                <tr>
                                  <th
                                    style={{
                                      textAlign: "left",
                                      padding: "8px",
                                      borderBottom: "1px solid #e0e0e0",
                                      color: "#525252",
                                      fontWeight: 500,
                                    }}
                                  >
                                    Parameter
                                  </th>
                                  <th
                                    style={{
                                      textAlign: "left",
                                      padding: "8px",
                                      borderBottom: "1px solid #e0e0e0",
                                      color: "#525252",
                                      fontWeight: 500,
                                    }}
                                  >
                                    Value
                                  </th>
                                </tr>
                              </thead>
                              <tbody>
                                {params.map((p, i) => (
                                  <tr key={i}>
                                    <td
                                      style={{
                                        padding: "8px",
                                        borderBottom: "1px solid #f4f4f4",
                                        fontFamily: "'IBM Plex Mono', monospace",
                                        fontSize: "12px",
                                        color: "#161616",
                                      }}
                                    >
                                      {p.key}
                                    </td>
                                    <td
                                      style={{
                                        padding: "8px",
                                        borderBottom: "1px solid #f4f4f4",
                                        color: "#161616",
                                      }}
                                    >
                                      {p.value}
                                    </td>
                                  </tr>
                                ))}
                              </tbody>
                            </table>
                          </div>
                        );
                      })()}

                      {/* Results Section */}
                      {(() => {
                        const results = parseResultsSection(selectedExp.content);
                        if (!results) return null;
                        return (
                          <div>
                            <div
                              style={{
                                fontSize: "12px",
                                fontWeight: 600,
                                color: "#161616",
                                marginBottom: "8px",
                              }}
                            >
                              Results
                            </div>
                            <div
                              style={{
                                fontFamily: "'IBM Plex Mono', monospace",
                                fontSize: "12px",
                                color: "#525252",
                                whiteSpace: "pre-wrap",
                                lineHeight: "20px",
                                backgroundColor: "#f4f4f4",
                                padding: "12px",
                                border: "1px solid #e0e0e0",
                              }}
                            >
                              {results}
                            </div>
                          </div>
                        );
                      })()}
                    </>
                  )}
                </div>
              </div>
            )}
          </>
        )}
      </div>
    </div>
  );
}
