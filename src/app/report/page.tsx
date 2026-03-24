"use client";

import { useEffect, useState, useCallback } from "react";
import Nav from "@/components/nav";

interface MissionData {
  id: string;
  phase: string;
  objective: string;
  createdAt: string;
  completedAt?: string;
  shareToken?: string;
  estimatedCost?: string;
}

interface Finding {
  id: string;
  title: string;
  impact: string;
  content: string;
}

interface ReportData {
  objective: string;
  challenge: string;
  findings: Finding[];
  summary: {
    duration: string | null;
    hypothesesTested: number;
    experimentsRun: number;
    findingsProduced: number;
    challengeReviews: number;
  };
  isComplete: boolean;
}

interface StatusResponse {
  mission: MissionData | null;
  kb: {
    artifacts: Array<{
      type: string;
      id: string;
      title: string;
      content: string;
      metadata: Record<string, string>;
    }>;
    artifactCounts: {
      hypotheses: number;
      experiments: number;
      findings: number;
      challengeReviews: number;
      tasks: number;
    };
  } | null;
}

function formatDuration(fromISO: string, toISO?: string): string {
  const ms =
    (toISO ? new Date(toISO).getTime() : Date.now()) -
    new Date(fromISO).getTime();
  const totalMins = Math.floor(ms / 60000);
  const hours = Math.floor(totalMins / 60);
  const mins = totalMins % 60;
  if (hours >= 24) {
    const days = Math.floor(hours / 24);
    const remHours = hours % 24;
    return `${days}d ${remHours}h ${mins}m`;
  }
  if (hours > 0) return `${hours}h ${mins}m`;
  return `${mins}m`;
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

export default function ReportPage() {
  const [status, setStatus] = useState<StatusResponse | null>(null);
  const [report, setReport] = useState<ReportData | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [shareUrl, setShareUrl] = useState<string | null>(null);
  const [shareLoading, setShareLoading] = useState(false);
  const [linkCopied, setLinkCopied] = useState(false);

  const fetchStatus = useCallback(async () => {
    try {
      const statusRes = await fetch("/api/status");
      if (!statusRes.ok) throw new Error("Failed to load status");
      const statusData: StatusResponse = await statusRes.json();
      setStatus(statusData);

      if (statusData.mission?.shareToken) {
        setShareUrl(
          `${window.location.origin}/api/reports/${statusData.mission.shareToken}`
        );
      }

      // Build report from KB data directly
      if (statusData.kb) {
        const findings = statusData.kb.artifacts
          .filter((a) => a.type === "finding")
          .sort((a, b) => {
            const impactOrder: Record<string, number> = {
              HIGH: 3,
              MEDIUM: 2,
              LOW: 1,
            };
            const aImpact =
              impactOrder[
                (a.metadata.impact || "").toUpperCase()
              ] || 0;
            const bImpact =
              impactOrder[
                (b.metadata.impact || "").toUpperCase()
              ] || 0;
            return bImpact - aImpact;
          })
          .map((f) => ({
            id: f.id,
            title: f.title,
            impact: f.metadata.impact || "unknown",
            content: f.content,
          }));

        let duration: string | null = null;
        if (
          statusData.mission?.createdAt &&
          statusData.mission?.completedAt
        ) {
          const hours =
            (new Date(statusData.mission.completedAt).getTime() -
              new Date(statusData.mission.createdAt).getTime()) /
            (1000 * 60 * 60);
          duration = `${hours.toFixed(1)}h`;
        }

        setReport({
          objective: statusData.mission?.objective || "",
          challenge: "",
          findings,
          summary: {
            duration,
            hypothesesTested:
              statusData.kb.artifactCounts.hypotheses,
            experimentsRun:
              statusData.kb.artifactCounts.experiments,
            findingsProduced:
              statusData.kb.artifactCounts.findings,
            challengeReviews:
              statusData.kb.artifactCounts.challengeReviews,
          },
          isComplete: statusData.mission?.phase === "COMPLETED",
        });
      }

      setError(null);
    } catch (err) {
      if (loading) {
        setError(err instanceof Error ? err.message : "Failed to load");
      }
    } finally {
      setLoading(false);
    }
  }, [loading]);

  useEffect(() => {
    fetchStatus();
    const interval = setInterval(fetchStatus, 5000);
    return () => clearInterval(interval);
  }, [fetchStatus]);

  async function handleShare() {
    if (!status?.mission?.id) return;
    setShareLoading(true);
    try {
      const res = await fetch(
        `/api/missions/${status.mission.id}/share`,
        { method: "POST" }
      );
      if (res.ok) {
        const data = await res.json();
        setShareUrl(data.shareUrl);
        await navigator.clipboard.writeText(data.shareUrl);
        setLinkCopied(true);
        setTimeout(() => setLinkCopied(false), 3000);
      }
    } catch {
      // Silently handle
    }
    setShareLoading(false);
  }

  // Style constants
  const card: React.CSSProperties = {
    backgroundColor: "#ffffff",
    border: "1px solid #e0e0e0",
    padding: 24,
    fontFamily: "'IBM Plex Sans', sans-serif",
  };

  const sectionLabel: React.CSSProperties = {
    fontSize: 12,
    fontWeight: 600,
    textTransform: "uppercase",
    letterSpacing: "0.04em",
    color: "#525252",
    marginBottom: 12,
  };

  if (loading) {
    return (
      <div
        style={{
          minHeight: "100vh", marginLeft: 64,
          backgroundColor: "#f4f4f4",
          fontFamily: "'IBM Plex Sans', sans-serif",
        }}
      >
        <Nav activePath="/report" />
        <div style={{ maxWidth: 960, margin: "0 auto", padding: 24 }}>
          <div
            style={{
              ...card,
              textAlign: "center",
              padding: 48,
              fontSize: 14,
              color: "#525252",
            }}
          >
            Loading report data...
          </div>
        </div>
      </div>
    );
  }

  if (error) {
    return (
      <div
        style={{
          minHeight: "100vh", marginLeft: 64,
          backgroundColor: "#f4f4f4",
          fontFamily: "'IBM Plex Sans', sans-serif",
        }}
      >
        <Nav activePath="/report" />
        <div style={{ maxWidth: 960, margin: "0 auto", padding: 24 }}>
          <div
            style={{
              ...card,
              textAlign: "center",
              padding: 48,
              fontSize: 14,
              color: "#525252",
            }}
          >
            {error}
          </div>
        </div>
      </div>
    );
  }

  const mission = status?.mission;
  const isComplete = mission?.phase === "COMPLETED";

  return (
    <div
      style={{
        minHeight: "100vh", marginLeft: 64,
        backgroundColor: "#f4f4f4",
        fontFamily: "'IBM Plex Sans', sans-serif",
      }}
    >
      <Nav activePath="/report" />

      <main
        style={{
          maxWidth: 960,
          margin: "0 auto",
          padding: "24px 24px 48px",
          display: "flex",
          flexDirection: "column",
          gap: 24,
        }}
      >
        <h1
          style={{
            fontSize: 20,
            fontWeight: 600,
            color: "#161616",
            marginBottom: 0,
          }}
        >
          Shareable Report
        </h1>

        {/* Mission in progress banner */}
        {mission && !isComplete && (
          <div
            style={{
              ...card,
              borderColor: "#f1c21b",
              borderWidth: 2,
              backgroundColor: "#fcf4d6",
            }}
          >
            <p
              style={{ fontSize: 14, fontWeight: 500, color: "#161616" }}
            >
              Mission in progress — this report will update as the agent
              produces findings.
            </p>
            <p style={{ fontSize: 12, color: "#525252", marginTop: 4 }}>
              Elapsed: {formatDuration(mission.createdAt)}
            </p>
          </div>
        )}

        {/* Executive Summary */}
        <div style={card}>
          <div style={sectionLabel}>Executive Summary</div>
          <p style={{ fontSize: 14, color: "#161616", lineHeight: "22px" }}>
            {mission?.objective || "No mission objective available."}
          </p>
        </div>

        {/* Key Findings */}
        <div>
          <div style={sectionLabel}>Key Findings</div>
          {report && report.findings.length > 0 ? (
            <div
              style={{ display: "flex", flexDirection: "column", gap: 8 }}
            >
              {report.findings.map((f) => {
                const impactKey = f.impact.toUpperCase();
                return (
                  <div
                    key={f.id}
                    style={{
                      ...card,
                      borderLeft: `3px solid ${IMPACT_COLORS[impactKey] || "#525252"}`,
                    }}
                  >
                    <div
                      style={{
                        display: "flex",
                        alignItems: "center",
                        gap: 8,
                        marginBottom: 8,
                      }}
                    >
                      <span
                        style={{
                          fontFamily: "'IBM Plex Mono', monospace",
                          fontSize: 12,
                          fontWeight: 600,
                          color: "#198038",
                        }}
                      >
                        {f.id}
                      </span>
                      <span
                        style={{
                          fontSize: 11,
                          fontWeight: 500,
                          color:
                            IMPACT_TEXT_COLORS[impactKey] || "#ffffff",
                          backgroundColor:
                            IMPACT_COLORS[impactKey] || "#525252",
                          padding: "1px 8px",
                          borderRadius: 2,
                        }}
                      >
                        {f.impact}
                      </span>
                    </div>
                    <p
                      style={{
                        fontSize: 14,
                        fontWeight: 500,
                        color: "#161616",
                        marginBottom: 8,
                      }}
                    >
                      {f.title}
                    </p>
                    <details>
                      <summary
                        style={{
                          fontSize: 13,
                          color: "#525252",
                          cursor: "pointer",
                        }}
                      >
                        View evidence
                      </summary>
                      <pre
                        style={{
                          whiteSpace: "pre-wrap",
                          marginTop: 8,
                          fontFamily: "'IBM Plex Mono', monospace",
                          fontSize: 12,
                          color: "#525252",
                          borderTop: "1px solid #e0e0e0",
                          paddingTop: 8,
                          maxHeight: 400,
                          overflow: "auto",
                        }}
                      >
                        {f.content.slice(0, 2000)}
                      </pre>
                    </details>
                  </div>
                );
              })}
            </div>
          ) : (
            <div style={{ ...card, textAlign: "center", padding: 48 }}>
              <p style={{ fontSize: 14, color: "#525252" }}>
                No findings yet. Key findings will appear here as the agent
                completes its research.
              </p>
            </div>
          )}
        </div>

        {/* Research Summary Stats */}
        {report && (
          <div style={card}>
            <div style={sectionLabel}>Research Summary</div>
            <div
              style={{
                display: "grid",
                gridTemplateColumns: "1fr 1fr 1fr 1fr",
                gap: 24,
              }}
            >
              <div>
                <div style={{ fontSize: 12, color: "#525252", marginBottom: 4 }}>
                  Duration
                </div>
                <div style={{ fontSize: 14, fontWeight: 500, color: "#161616" }}>
                  {report.summary.duration ||
                    (mission
                      ? formatDuration(
                          mission.createdAt,
                          mission.completedAt
                        )
                      : "N/A")}
                </div>
              </div>
              <div>
                <div style={{ fontSize: 12, color: "#525252", marginBottom: 4 }}>
                  Hypotheses
                </div>
                <div style={{ fontSize: 14, fontWeight: 500, color: "#161616" }}>
                  {report.summary.hypothesesTested}
                </div>
              </div>
              <div>
                <div style={{ fontSize: 12, color: "#525252", marginBottom: 4 }}>
                  Experiments
                </div>
                <div style={{ fontSize: 14, fontWeight: 500, color: "#161616" }}>
                  {report.summary.experimentsRun}
                </div>
              </div>
              <div>
                <div style={{ fontSize: 12, color: "#525252", marginBottom: 4 }}>
                  Findings
                </div>
                <div style={{ fontSize: 14, fontWeight: 500, color: "#161616" }}>
                  {report.summary.findingsProduced}
                </div>
              </div>
            </div>
          </div>
        )}

        {/* Share Section */}
        <div style={card}>
          <div style={sectionLabel}>Share</div>
          {isComplete ? (
            <div>
              <button
                onClick={handleShare}
                disabled={shareLoading}
                style={{
                  padding: "8px 16px",
                  border: "1px solid #e0e0e0",
                  backgroundColor: shareLoading ? "#f4f4f4" : "#ffffff",
                  color: "#161616",
                  fontSize: 14,
                  fontWeight: 500,
                  fontFamily: "'IBM Plex Sans', sans-serif",
                  cursor: shareLoading ? "wait" : "pointer",
                }}
              >
                {linkCopied
                  ? "Link copied!"
                  : shareUrl
                    ? "Copy link again"
                    : "Generate public link"}
              </button>
              {shareUrl && (
                <p style={{ fontSize: 12, color: "#525252", marginTop: 8 }}>
                  Public link:{" "}
                  <code
                    style={{
                      backgroundColor: "#f4f4f4",
                      padding: "2px 6px",
                      fontSize: 12,
                      fontFamily: "'IBM Plex Mono', monospace",
                    }}
                  >
                    {shareUrl}
                  </code>
                </p>
              )}
            </div>
          ) : (
            <p style={{ fontSize: 14, color: "#525252" }}>
              Share links can only be generated for completed missions.
            </p>
          )}
        </div>
      </main>
    </div>
  );
}
