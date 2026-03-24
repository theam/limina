"use client";

import { useEffect, useState, useCallback } from "react";
import Nav from "../../components/nav";
import StatusBadge from "../../components/status-badge";
import Link from "next/link";

interface Artifact {
  type: string;
  id: string;
  title: string;
  status: string;
  metadata: Record<string, string>;
  content: string;
}

interface TaskEntry {
  id: string;
  title: string;
  status: string;
  priority: string;
  type: string;
}

interface StatusResponse {
  mission: {
    id: string;
    phase: string;
    objective: string;
  };
  kb: {
    artifacts: Artifact[];
    tasks: TaskEntry[];
    phase: string;
    artifactCounts: {
      hypotheses: number;
      experiments: number;
      findings: number;
      challengeReviews: number;
      tasks: number;
    };
  };
}

const STATUS_COLORS: Record<string, string> = {
  CONFIRMED: "#198038",
  REJECTED: "#da1e28",
  TESTING: "#0f62fe",
  PROPOSED: "#525252",
};

const TYPE_BORDER_COLORS: Record<string, string> = {
  hypothesis: "#0f62fe",
  experiment: "#d2a106",
  finding: "#198038",
  challenge_review: "#8a3ffc",
  strategic_review: "#8a3ffc",
};

function statusBadge(status: string) {
  const color = STATUS_COLORS[status.toUpperCase()] || "#525252";
  return (
    <span
      style={{
        display: "inline-block",
        fontSize: "11px",
        fontWeight: 500,
        fontFamily: "'IBM Plex Sans', sans-serif",
        color: "#ffffff",
        backgroundColor: color,
        padding: "1px 8px",
        borderRadius: "2px",
        lineHeight: "18px",
      }}
    >
      {status}
    </span>
  );
}

function impactBadge(impact: string) {
  const colors: Record<string, string> = {
    HIGH: "#198038",
    MEDIUM: "#f1c21b",
    LOW: "#525252",
  };
  const textColors: Record<string, string> = {
    HIGH: "#ffffff",
    MEDIUM: "#161616",
    LOW: "#ffffff",
  };
  const key = impact.toUpperCase();
  return (
    <span
      style={{
        display: "inline-block",
        fontSize: "11px",
        fontWeight: 500,
        fontFamily: "'IBM Plex Sans', sans-serif",
        color: textColors[key] || "#ffffff",
        backgroundColor: colors[key] || "#525252",
        padding: "1px 8px",
        borderRadius: "2px",
        lineHeight: "18px",
      }}
    >
      {impact}
    </span>
  );
}

/** Build tree: Task → Hypotheses → Experiments → Findings */
function buildTree(
  tasks: TaskEntry[],
  artifacts: Artifact[]
) {
  const hypotheses = artifacts.filter((a) => a.type === "hypothesis");
  const experiments = artifacts.filter((a) => a.type === "experiment");
  const findings = artifacts.filter((a) => a.type === "finding");
  const reviews = artifacts.filter(
    (a) => a.type === "challenge_review" || a.type === "strategic_review"
  );

  // Link experiments to hypotheses via metadata "hypothesis"
  // Link findings to experiments via metadata "experiment"
  const expsByHypothesis: Record<string, Artifact[]> = {};
  for (const exp of experiments) {
    const hId = exp.metadata.hypothesis || "unlinked";
    if (!expsByHypothesis[hId]) expsByHypothesis[hId] = [];
    expsByHypothesis[hId].push(exp);
  }

  const findingsByExperiment: Record<string, Artifact[]> = {};
  for (const f of findings) {
    const eId = f.metadata.experiment || "unlinked";
    if (!findingsByExperiment[eId]) findingsByExperiment[eId] = [];
    findingsByExperiment[eId].push(f);
  }

  // Link hypotheses to tasks via metadata "task"
  const hypothesesByTask: Record<string, Artifact[]> = {};
  for (const h of hypotheses) {
    const tId = h.metadata.task || "unlinked";
    if (!hypothesesByTask[tId]) hypothesesByTask[tId] = [];
    hypothesesByTask[tId].push(h);
  }

  return { hypothesesByTask, expsByHypothesis, findingsByExperiment, reviews, tasks };
}

export default function ResearchPage() {
  const [data, setData] = useState<StatusResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [expandedNodes, setExpandedNodes] = useState<Set<string>>(new Set());

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

  function toggleNode(id: string) {
    setExpandedNodes((prev) => {
      const next = new Set(prev);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return next;
    });
  }

  const isEmpty =
    !data ||
    (data.kb.artifacts.filter((a) =>
      ["hypothesis", "experiment", "finding", "challenge_review", "strategic_review"].includes(a.type)
    ).length === 0 &&
      data.kb.tasks.length === 0);

  const tree = data
    ? buildTree(data.kb.tasks, data.kb.artifacts)
    : null;

  // Determine which tasks to show: real tasks or synthetic "unlinked"
  const taskIds = tree
    ? [
        ...tree.tasks.map((t) => t.id),
        ...(tree.hypothesesByTask["unlinked"] ? ["unlinked"] : []),
      ]
    : [];

  return (
    <div style={{ minHeight: "100vh", marginLeft: 64, backgroundColor: "#f4f4f4", fontFamily: "'IBM Plex Sans', sans-serif" }}>
      <Nav activePath="/research" />

      <div style={{ maxWidth: "960px", margin: "0 auto", padding: "24px" }}>
        <h1
          style={{
            fontSize: "20px",
            fontWeight: 600,
            color: "#161616",
            marginBottom: "8px",
          }}
        >
          Research Graph
        </h1>
        <p style={{ fontSize: "14px", color: "#525252", marginBottom: "24px" }}>
          Hypothesis &rarr; Experiment &rarr; Finding traceability
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
            Loading research data...
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
            Research beginning. Hypotheses will appear as the agent investigates.
          </div>
        )}

        {!loading && !isEmpty && tree && (
          <>
            {/* Tasks with nested H → E → F */}
            {taskIds.map((taskId) => {
              const task = tree.tasks.find((t) => t.id === taskId);
              const taskHypotheses = tree.hypothesesByTask[taskId] || [];

              return (
                <div
                  key={taskId}
                  style={{ marginBottom: "16px" }}
                >
                  {/* Task header */}
                  <div
                    style={{
                      backgroundColor: "#ffffff",
                      border: "1px solid #e0e0e0",
                      padding: "12px 16px",
                      display: "flex",
                      alignItems: "center",
                      gap: "8px",
                    }}
                  >
                    <span
                      style={{
                        fontFamily: "'IBM Plex Mono', monospace",
                        fontSize: "13px",
                        fontWeight: 500,
                        color: "#161616",
                      }}
                    >
                      {taskId === "unlinked" ? "Unlinked" : taskId}
                    </span>
                    {task && (
                      <>
                        <Link href={`/artifact/${taskId}`} style={{ fontSize: "14px", color: "#161616", flex: 1, textDecoration: "none" }}>
                          {task.title}
                        </Link>
                        <StatusBadge status={task.status} />
                      </>
                    )}
                    {!task && taskId === "unlinked" && (
                      <span style={{ fontSize: "14px", color: "#525252", flex: 1 }}>
                        Hypotheses not linked to a task
                      </span>
                    )}
                  </div>

                  {/* Hypotheses under this task */}
                  {taskHypotheses.map((h) => {
                    const hExpanded = expandedNodes.has(h.id);
                    const hExperiments = tree.expsByHypothesis[h.id] || [];

                    return (
                      <div key={h.id} style={{ marginLeft: "24px" }}>
                        {/* Hypothesis node */}
                        <div
                          onClick={() => toggleNode(h.id)}
                          style={{
                            backgroundColor: "#ffffff",
                            border: "1px solid #e0e0e0",
                            borderLeft: `3px solid ${TYPE_BORDER_COLORS.hypothesis}`,
                            padding: "10px 16px",
                            marginTop: "4px",
                            cursor: "pointer",
                            display: "flex",
                            alignItems: "center",
                            gap: "8px",
                          }}
                        >
                          <span style={{ fontSize: "12px", color: "#525252" }}>
                            {hExpanded ? "\u25BC" : "\u25B6"}
                          </span>
                          <span
                            style={{
                              fontFamily: "'IBM Plex Mono', monospace",
                              fontSize: "12px",
                              fontWeight: 500,
                              color: "#0f62fe",
                            }}
                          >
                            <Link href={`/artifact/${h.id}`} style={{color: "inherit", textDecoration: "none"}}>{h.id}</Link>
                          </span>
                          <span style={{ fontSize: "14px", color: "#161616", flex: 1 }}>
                            <Link href={`/artifact/${h.id}`} style={{color: "#161616", textDecoration: "none"}} onClick={(e) => e.stopPropagation()}>{h.title}</Link>
                          </span>
                          <StatusBadge status={h.status} />
                        </div>

                        {/* Expanded content */}
                        {hExpanded && (
                          <div
                            style={{
                              backgroundColor: "#f4f4f4",
                              borderLeft: `3px solid ${TYPE_BORDER_COLORS.hypothesis}`,
                              borderRight: "1px solid #e0e0e0",
                              borderBottom: "1px solid #e0e0e0",
                              padding: "12px 16px",
                              marginLeft: "0px",
                              fontSize: "13px",
                              color: "#525252",
                              fontFamily: "'IBM Plex Mono', monospace",
                              whiteSpace: "pre-wrap",
                              maxHeight: "400px",
                              overflow: "auto",
                            }}
                          >
                            {h.content.slice(0, 3000)}
                          </div>
                        )}

                        {/* Experiments under this hypothesis */}
                        {hExperiments.map((e) => {
                          const eExpanded = expandedNodes.has(e.id);
                          const eFindings = tree.findingsByExperiment[e.id] || [];

                          return (
                            <div key={e.id} style={{ marginLeft: "24px" }}>
                              {/* Experiment node */}
                              <div
                                onClick={() => toggleNode(e.id)}
                                style={{
                                  backgroundColor: "#ffffff",
                                  border: "1px solid #e0e0e0",
                                  borderLeft: `3px solid ${TYPE_BORDER_COLORS.experiment}`,
                                  padding: "10px 16px",
                                  marginTop: "4px",
                                  cursor: "pointer",
                                  display: "flex",
                                  alignItems: "center",
                                  gap: "8px",
                                }}
                              >
                                <span style={{ fontSize: "12px", color: "#525252" }}>
                                  {eExpanded ? "\u25BC" : "\u25B6"}
                                </span>
                                <span
                                  style={{
                                    fontFamily: "'IBM Plex Mono', monospace",
                                    fontSize: "12px",
                                    fontWeight: 500,
                                    color: "#d2a106",
                                  }}
                                >
                                  <Link href={`/artifact/${e.id}`} style={{color: "inherit", textDecoration: "none"}}>{e.id}</Link>
                                </span>
                                <span style={{ fontSize: "14px", color: "#161616", flex: 1 }}>
                                  <Link href={`/artifact/${e.id}`} style={{color: "#161616", textDecoration: "none"}} onClick={(e) => e.stopPropagation()}>{e.title}</Link>
                                </span>
                                <StatusBadge status={e.status} />
                              </div>

                              {/* Expanded experiment content */}
                              {eExpanded && (
                                <div
                                  style={{
                                    backgroundColor: "#f4f4f4",
                                    borderLeft: `3px solid ${TYPE_BORDER_COLORS.experiment}`,
                                    borderRight: "1px solid #e0e0e0",
                                    borderBottom: "1px solid #e0e0e0",
                                    padding: "12px 16px",
                                    fontSize: "13px",
                                    color: "#525252",
                                    fontFamily: "'IBM Plex Mono', monospace",
                                    whiteSpace: "pre-wrap",
                                    maxHeight: "400px",
                                    overflow: "auto",
                                  }}
                                >
                                  {e.content.slice(0, 3000)}
                                </div>
                              )}

                              {/* Findings under this experiment */}
                              {eFindings.map((f) => {
                                const fExpanded = expandedNodes.has(f.id);
                                const impact = f.metadata.impact || f.metadata["impact level"] || "";

                                return (
                                  <div key={f.id} style={{ marginLeft: "24px" }}>
                                    {/* Finding node */}
                                    <div
                                      onClick={() => toggleNode(f.id)}
                                      style={{
                                        backgroundColor: "#ffffff",
                                        border: "1px solid #e0e0e0",
                                        borderLeft: `3px solid ${TYPE_BORDER_COLORS.finding}`,
                                        padding: "10px 16px",
                                        marginTop: "4px",
                                        cursor: "pointer",
                                        display: "flex",
                                        alignItems: "center",
                                        gap: "8px",
                                      }}
                                    >
                                      <span style={{ fontSize: "12px", color: "#525252" }}>
                                        {fExpanded ? "\u25BC" : "\u25B6"}
                                      </span>
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
                                        <Link href={`/artifact/${f.id}`} style={{color: "#161616", textDecoration: "none"}} onClick={(e) => e.stopPropagation()}>{f.title}</Link>
                                      </span>
                                      {impact && impactBadge(impact)}
                                      <StatusBadge status={f.status} />
                                    </div>

                                    {/* Expanded finding content */}
                                    {fExpanded && (
                                      <div
                                        style={{
                                          backgroundColor: "#f4f4f4",
                                          borderLeft: `3px solid ${TYPE_BORDER_COLORS.finding}`,
                                          borderRight: "1px solid #e0e0e0",
                                          borderBottom: "1px solid #e0e0e0",
                                          padding: "12px 16px",
                                          fontSize: "13px",
                                          color: "#525252",
                                          fontFamily: "'IBM Plex Mono', monospace",
                                          whiteSpace: "pre-wrap",
                                          maxHeight: "400px",
                                          overflow: "auto",
                                        }}
                                      >
                                        {f.content.slice(0, 3000)}
                                      </div>
                                    )}
                                  </div>
                                );
                              })}
                            </div>
                          );
                        })}
                      </div>
                    );
                  })}
                </div>
              );
            })}

            {/* Also show experiments and findings not linked to any hypothesis */}
            {(() => {
              const unlinkedExps = tree.expsByHypothesis["unlinked"] || [];
              if (unlinkedExps.length === 0) return null;
              return (
                <div style={{ marginBottom: "16px" }}>
                  <div
                    style={{
                      backgroundColor: "#ffffff",
                      border: "1px solid #e0e0e0",
                      padding: "12px 16px",
                      fontSize: "14px",
                      color: "#525252",
                      fontWeight: 500,
                    }}
                  >
                    Unlinked Experiments
                  </div>
                  {unlinkedExps.map((e) => {
                    const eExpanded = expandedNodes.has(e.id);
                    const eFindings = tree.findingsByExperiment[e.id] || [];

                    return (
                      <div key={e.id} style={{ marginLeft: "24px" }}>
                        <div
                          onClick={() => toggleNode(e.id)}
                          style={{
                            backgroundColor: "#ffffff",
                            border: "1px solid #e0e0e0",
                            borderLeft: `3px solid ${TYPE_BORDER_COLORS.experiment}`,
                            padding: "10px 16px",
                            marginTop: "4px",
                            cursor: "pointer",
                            display: "flex",
                            alignItems: "center",
                            gap: "8px",
                          }}
                        >
                          <span style={{ fontSize: "12px", color: "#525252" }}>
                            {eExpanded ? "\u25BC" : "\u25B6"}
                          </span>
                          <span
                            style={{
                              fontFamily: "'IBM Plex Mono', monospace",
                              fontSize: "12px",
                              fontWeight: 500,
                              color: "#d2a106",
                            }}
                          >
                            <Link href={`/artifact/${e.id}`} style={{color: "inherit", textDecoration: "none"}}>{e.id}</Link>
                          </span>
                          <span style={{ fontSize: "14px", color: "#161616", flex: 1 }}>
                            <Link href={`/artifact/${e.id}`} style={{color: "#161616", textDecoration: "none"}} onClick={(e) => e.stopPropagation()}>{e.title}</Link>
                          </span>
                          <StatusBadge status={e.status} />
                        </div>
                        {eExpanded && (
                          <div
                            style={{
                              backgroundColor: "#f4f4f4",
                              borderLeft: `3px solid ${TYPE_BORDER_COLORS.experiment}`,
                              borderRight: "1px solid #e0e0e0",
                              borderBottom: "1px solid #e0e0e0",
                              padding: "12px 16px",
                              fontSize: "13px",
                              color: "#525252",
                              fontFamily: "'IBM Plex Mono', monospace",
                              whiteSpace: "pre-wrap",
                              maxHeight: "400px",
                              overflow: "auto",
                            }}
                          >
                            {e.content.slice(0, 3000)}
                          </div>
                        )}
                        {eFindings.map((f) => {
                          const fExpanded = expandedNodes.has(f.id);
                          const impact = f.metadata.impact || f.metadata["impact level"] || "";
                          return (
                            <div key={f.id} style={{ marginLeft: "24px" }}>
                              <div
                                onClick={() => toggleNode(f.id)}
                                style={{
                                  backgroundColor: "#ffffff",
                                  border: "1px solid #e0e0e0",
                                  borderLeft: `3px solid ${TYPE_BORDER_COLORS.finding}`,
                                  padding: "10px 16px",
                                  marginTop: "4px",
                                  cursor: "pointer",
                                  display: "flex",
                                  alignItems: "center",
                                  gap: "8px",
                                }}
                              >
                                <span style={{ fontSize: "12px", color: "#525252" }}>
                                  {fExpanded ? "\u25BC" : "\u25B6"}
                                </span>
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
                                  <Link href={`/artifact/${f.id}`} style={{color: "#161616", textDecoration: "none"}} onClick={(e) => e.stopPropagation()}>{f.title}</Link>
                                </span>
                                {impact && impactBadge(impact)}
                                <StatusBadge status={f.status} />
                              </div>
                              {fExpanded && (
                                <div
                                  style={{
                                    backgroundColor: "#f4f4f4",
                                    borderLeft: `3px solid ${TYPE_BORDER_COLORS.finding}`,
                                    borderRight: "1px solid #e0e0e0",
                                    borderBottom: "1px solid #e0e0e0",
                                    padding: "12px 16px",
                                    fontSize: "13px",
                                    color: "#525252",
                                    fontFamily: "'IBM Plex Mono', monospace",
                                    whiteSpace: "pre-wrap",
                                    maxHeight: "400px",
                                    overflow: "auto",
                                  }}
                                >
                                  {f.content.slice(0, 3000)}
                                </div>
                              )}
                            </div>
                          );
                        })}
                      </div>
                    );
                  })}
                </div>
              );
            })()}

            {/* Unlinked findings (not linked to any experiment) */}
            {(() => {
              const unlinkedFindings = tree.findingsByExperiment["unlinked"] || [];
              if (unlinkedFindings.length === 0) return null;
              return (
                <div style={{ marginBottom: "16px" }}>
                  <div
                    style={{
                      backgroundColor: "#ffffff",
                      border: "1px solid #e0e0e0",
                      padding: "12px 16px",
                      fontSize: "14px",
                      color: "#525252",
                      fontWeight: 500,
                    }}
                  >
                    Unlinked Findings
                  </div>
                  {unlinkedFindings.map((f) => {
                    const fExpanded = expandedNodes.has(f.id);
                    const impact = f.metadata.impact || f.metadata["impact level"] || "";
                    return (
                      <div key={f.id} style={{ marginLeft: "24px" }}>
                        <div
                          onClick={() => toggleNode(f.id)}
                          style={{
                            backgroundColor: "#ffffff",
                            border: "1px solid #e0e0e0",
                            borderLeft: `3px solid ${TYPE_BORDER_COLORS.finding}`,
                            padding: "10px 16px",
                            marginTop: "4px",
                            cursor: "pointer",
                            display: "flex",
                            alignItems: "center",
                            gap: "8px",
                          }}
                        >
                          <span style={{ fontSize: "12px", color: "#525252" }}>
                            {fExpanded ? "\u25BC" : "\u25B6"}
                          </span>
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
                            <Link href={`/artifact/${f.id}`} style={{color: "#161616", textDecoration: "none"}} onClick={(e) => e.stopPropagation()}>{f.title}</Link>
                          </span>
                          {impact && impactBadge(impact)}
                          <StatusBadge status={f.status} />
                        </div>
                        {fExpanded && (
                          <div
                            style={{
                              backgroundColor: "#f4f4f4",
                              borderLeft: `3px solid ${TYPE_BORDER_COLORS.finding}`,
                              borderRight: "1px solid #e0e0e0",
                              borderBottom: "1px solid #e0e0e0",
                              padding: "12px 16px",
                              fontSize: "13px",
                              color: "#525252",
                              fontFamily: "'IBM Plex Mono', monospace",
                              whiteSpace: "pre-wrap",
                              maxHeight: "400px",
                              overflow: "auto",
                            }}
                          >
                            {f.content.slice(0, 3000)}
                          </div>
                        )}
                      </div>
                    );
                  })}
                </div>
              );
            })()}

            {/* Challenge Reviews section */}
            {tree.reviews.length > 0 && (
              <div style={{ marginTop: "32px" }}>
                <h2
                  style={{
                    fontSize: "16px",
                    fontWeight: 600,
                    color: "#161616",
                    marginBottom: "8px",
                  }}
                >
                  Challenge Reviews
                </h2>
                {tree.reviews.map((r) => {
                  const rExpanded = expandedNodes.has(r.id);
                  return (
                    <div key={r.id} style={{ marginBottom: "4px" }}>
                      <div
                        onClick={() => toggleNode(r.id)}
                        style={{
                          backgroundColor: "#ffffff",
                          border: "1px solid #e0e0e0",
                          borderLeft: `3px solid ${TYPE_BORDER_COLORS.challenge_review}`,
                          padding: "10px 16px",
                          cursor: "pointer",
                          display: "flex",
                          alignItems: "center",
                          gap: "8px",
                        }}
                      >
                        <span style={{ fontSize: "12px", color: "#525252" }}>
                          {rExpanded ? "\u25BC" : "\u25B6"}
                        </span>
                        <span
                          style={{
                            fontFamily: "'IBM Plex Mono', monospace",
                            fontSize: "12px",
                            fontWeight: 500,
                            color: "#8a3ffc",
                          }}
                        >
                          <Link href={`/artifact/${r.id}`} style={{color: "inherit", textDecoration: "none"}}>{r.id}</Link>
                        </span>
                        <span style={{ fontSize: "14px", color: "#161616", flex: 1 }}>
                          {r.title}
                        </span>
                        <StatusBadge status={r.status} />
                      </div>
                      {rExpanded && (
                        <div
                          style={{
                            backgroundColor: "#f4f4f4",
                            borderLeft: `3px solid ${TYPE_BORDER_COLORS.challenge_review}`,
                            borderRight: "1px solid #e0e0e0",
                            borderBottom: "1px solid #e0e0e0",
                            padding: "12px 16px",
                            fontSize: "13px",
                            color: "#525252",
                            fontFamily: "'IBM Plex Mono', monospace",
                            whiteSpace: "pre-wrap",
                            maxHeight: "400px",
                            overflow: "auto",
                          }}
                        >
                          {r.content.slice(0, 3000)}
                        </div>
                      )}
                    </div>
                  );
                })}
              </div>
            )}
          </>
        )}

        {/* Browsable artifact sections — all types */}
        {!loading && !isEmpty && data?.kb?.artifacts && data.kb.artifacts.length > 0 && (
          <>
            {[
              { type: "hypothesis", label: "Hypotheses", color: "#0f62fe" },
              { type: "experiment", label: "Experiments", color: "#f1c21b" },
              { type: "finding", label: "Findings", color: "#198038" },
              { type: "literature", label: "Literature", color: "#005d5d" },
              { type: "challenge_review", label: "Challenge Reviews", color: "#8a3ffc" },
              { type: "strategic_review", label: "Strategic Reviews", color: "#8a3ffc" },
              { type: "report", label: "Reports", color: "#8a3ffc" },
            ].map(({ type: artType, label: sectionLabel, color: sectionColor }) => {
              const items = data!.kb!.artifacts.filter((a) => a.type === artType);
              if (items.length === 0) return null;
              return (
                <div key={artType} style={{ marginTop: "32px" }}>
                  <div
                    style={{
                      fontSize: "12px",
                      fontWeight: 600,
                      textTransform: "uppercase",
                      letterSpacing: "0.04em",
                      color: "#525252",
                      marginBottom: "12px",
                    }}
                  >
                    {sectionLabel} ({items.length})
                  </div>
                  <div style={{ display: "flex", flexDirection: "column", gap: "4px" }}>
                    {items.map((a) => (
                      <Link
                        key={a.id}
                        href={`/artifact/${a.id}`}
                        style={{
                          display: "flex",
                          alignItems: "center",
                          gap: "12px",
                          padding: "12px 16px",
                          backgroundColor: "#ffffff",
                          border: "1px solid #e0e0e0",
                          borderLeft: `3px solid ${sectionColor}`,
                          borderRadius: "4px",
                          textDecoration: "none",
                          transition: "border-color 150ms",
                        }}
                      >
                        <span
                          style={{
                            fontSize: "12px",
                            fontWeight: 600,
                            fontFamily: "'IBM Plex Mono', monospace",
                            color: sectionColor,
                            minWidth: "48px",
                          }}
                        >
                          {a.id}
                        </span>
                        <span style={{ flex: 1, fontSize: "14px", color: "#161616" }}>
                          {a.title}
                        </span>
                        <StatusBadge status={a.status} />
                      </Link>
                    ))}
                  </div>
                </div>
              );
            })}
          </>
        )}
      </div>
    </div>
  );
}
