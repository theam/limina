"use client";

import { useEffect, useState, useCallback } from "react";
import Link from "next/link";
import Nav from "../components/nav";

interface MissionData {
  id: string;
  phase: string;
  objective: string;
  createdAt: string;
  completedAt?: string;
  budget?: string;
  estimatedCost?: string;
  estimatedDuration?: string;
  failureReason?: string;
}

interface ArtifactCounts {
  hypotheses: number;
  experiments: number;
  findings: number;
  challengeReviews: number;
  tasks: number;
}

interface Artifact {
  type: string;
  id: string;
  title: string;
  status: string;
  filePath: string;
  lastModified?: string;
}

interface CeoRequest {
  id: string;
  title: string;
  status: "PENDING" | "RESOLVED" | "AUTO_RESOLVED";
  description: string;
  date?: string;
}

interface KBData {
  artifacts: Artifact[];
  tasks: Array<{ id: string; title: string; status: string }>;
  ceoRequests: CeoRequest[];
  phase: string;
  artifactCounts: ArtifactCounts;
}

interface StatusResponse {
  mission: MissionData | null;
  kb: KBData | null;
  pendingEscalations: CeoRequest[];
}

type NavStatus = "Running" | "Completed" | "Failed" | "Idle";

const phaseLabels: Record<string, string> = {
  idle: "Idle",
  hypothesizing: "Hypothesizing",
  experimenting: "Experimenting",
  documenting: "Documenting",
  reviewing: "Reviewing",
};

const missionPhaseLabels: Record<string, string> = {
  CREATED: "Created",
  PLANNING: "Planning",
  PLAN_READY: "Plan Ready",
  RUNNING: "Running",
  CHECKPOINT_WAITING: "Checkpoint",
  ESCALATION_WAITING: "Escalation",
  STALLED: "Stalled",
  BLOCKED: "Blocked",
  FAILED_RECOVERABLE: "Failed",
  COMPLETED: "Completed",
  KILLED: "Killed",
};

const artifactTypeLabels: Record<string, string> = {
  hypothesis: "Hypothesis",
  experiment: "Experiment",
  finding: "Finding",
  challenge_review: "Challenge Review",
  strategic_review: "Strategic Review",
  task: "Task",
  literature: "Literature",
};

const artifactTypeColors: Record<string, string> = {
  hypothesis: "#0f62fe",
  experiment: "#8a3ffc",
  finding: "#198038",
  challenge_review: "#b28600",
  strategic_review: "#b28600",
  task: "#525252",
  literature: "#005d5d",
};

function deriveNavStatus(mission: MissionData | null): NavStatus {
  if (!mission) return "Idle";
  const phase = mission.phase;
  if (phase === "COMPLETED") return "Completed";
  if (phase === "FAILED_RECOVERABLE" || phase === "KILLED") return "Failed";
  if (
    phase === "RUNNING" ||
    phase === "PLANNING" ||
    phase === "CHECKPOINT_WAITING" ||
    phase === "ESCALATION_WAITING" ||
    phase === "STALLED"
  )
    return "Running";
  return "Idle";
}

function formatElapsed(createdAt: string): string {
  const diff = Date.now() - new Date(createdAt).getTime();
  const totalMinutes = Math.floor(diff / 60000);
  if (totalMinutes < 1) return "< 1 min";
  if (totalMinutes < 60) return `${totalMinutes}m`;
  const hours = Math.floor(totalMinutes / 60);
  const mins = totalMinutes % 60;
  if (hours < 24) return mins > 0 ? `${hours}h ${mins}m` : `${hours}h`;
  const days = Math.floor(hours / 24);
  const remHours = hours % 24;
  return remHours > 0 ? `${days}d ${remHours}h` : `${days}d`;
}

// Phase progress: maps research phase to a 0-4 step index
const phaseSteps = ["idle", "hypothesizing", "experimenting", "documenting", "reviewing"];

export default function Dashboard() {
  const [data, setData] = useState<StatusResponse | null>(null);
  const [error, setError] = useState(false);
  const [loading, setLoading] = useState(true);

  const fetchStatus = useCallback(async () => {
    try {
      const res = await fetch("/api/status");
      if (!res.ok) throw new Error("fetch failed");
      const json: StatusResponse = await res.json();
      setData(json);
      setError(false);
    } catch {
      setError(true);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchStatus();
    const interval = setInterval(fetchStatus, 5000);
    return () => clearInterval(interval);
  }, [fetchStatus]);

  const mission = data?.mission ?? null;
  const kb = data?.kb ?? null;
  const pendingEscalations = data?.pendingEscalations ?? [];
  const navStatus = deriveNavStatus(mission);
  const costSnapshot = mission?.estimatedCost ?? undefined;

  const totalArtifacts = kb
    ? kb.artifactCounts.hypotheses +
      kb.artifactCounts.experiments +
      kb.artifactCounts.findings +
      kb.artifactCounts.challengeReviews
    : 0;

  // Sort artifacts chronologically (by ID number descending as proxy)
  const recentArtifacts = kb
    ? [...kb.artifacts]
        .sort((a, b) => {
          const numA = parseInt(a.id.replace(/\D/g, ""), 10) || 0;
          const numB = parseInt(b.id.replace(/\D/g, ""), 10) || 0;
          return numB - numA;
        })
        .slice(0, 10)
    : [];

  const phaseIndex = kb ? phaseSteps.indexOf(kb.phase) : 0;

  // Styles
  const card: React.CSSProperties = {
    backgroundColor: "#ffffff",
    border: "1px solid var(--border-subtle, #e0e0e0)",
    borderRadius: 8,
    padding: 24,
    fontFamily: "'IBM Plex Sans', sans-serif",
  };

  const sectionTitle: React.CSSProperties = {
    fontSize: 12,
    fontWeight: 600,
    textTransform: "uppercase" as const,
    letterSpacing: "0.04em",
    color: "var(--text-secondary, #525252)",
    marginBottom: 16,
  };

  const label: React.CSSProperties = {
    fontSize: 12,
    color: "var(--text-secondary, #525252)",
    marginBottom: 4,
  };

  const value: React.CSSProperties = {
    fontSize: 14,
    fontWeight: 500,
    color: "var(--text-primary, #161616)",
  };

  return (
    <div
      style={{
        minHeight: "100vh", marginLeft: 64,
        backgroundColor: "#f4f4f4",
        fontFamily: "'IBM Plex Sans', sans-serif",
      }}
    >
      <Nav activePath="/" />

      <main
        style={{
          maxWidth: 1200,
          margin: "0 auto",
          padding: "24px 24px 48px",
          display: "flex",
          flexDirection: "column",
          gap: 24,
        }}
      >
        {/* Error state */}
        {error && !loading && (
          <div style={{ ...card, textAlign: "center", padding: 48 }}>
            <p
              style={{
                fontSize: 14,
                color: "var(--text-secondary, #525252)",
                marginBottom: 16,
              }}
            >
              Research agent is not running.
            </p>
            <button
              onClick={fetchStatus}
              style={{
                fontSize: 13,
                color: "var(--interactive, #0f62fe)",
                background: "none",
                border: "none",
                cursor: "pointer",
                textDecoration: "underline",
              }}
            >
              Retry
            </button>
          </div>
        )}

        {/* Loading skeleton */}
        {loading && (
          <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
            {[1, 2, 3].map((i) => (
              <div
                key={i}
                style={{
                  ...card,
                  height: 80,
                  opacity: 0.5,
                }}
              />
            ))}
          </div>
        )}

        {/* Main content */}
        {!loading && !error && data && (
          <>
            {/* Pending escalations (attention required) */}
            {pendingEscalations.length > 0 && (
              <div
                style={{
                  ...card,
                  borderColor: "#da1e28",
                  borderWidth: 2,
                  backgroundColor: "#fff1f1",
                }}
              >
                <div style={{ ...sectionTitle, color: "#da1e28" }}>
                  Attention Required
                </div>
                {pendingEscalations.map((req) => (
                  <div
                    key={req.id}
                    style={{
                      padding: "8px 0",
                      borderBottom: "1px solid #ffd7d9",
                    }}
                  >
                    <div
                      style={{
                        fontSize: 14,
                        fontWeight: 500,
                        color: "#161616",
                      }}
                    >
                      {req.id}: {req.title}
                    </div>
                    {req.description && (
                      <div
                        style={{
                          fontSize: 13,
                          color: "#525252",
                          marginTop: 4,
                        }}
                      >
                        {req.description}
                      </div>
                    )}
                    {req.date && (
                      <div
                        style={{
                          fontSize: 12,
                          color: "#8d8d8d",
                          marginTop: 4,
                        }}
                      >
                        {req.date}
                      </div>
                    )}
                  </div>
                ))}
              </div>
            )}

            {/* Mission overview */}
            {mission ? (
              <div style={card}>
                <div style={sectionTitle}>Mission Overview</div>
                {/* Objective as full-width heading — not crammed into a grid cell */}
                <div style={{ marginBottom: 20 }}>
                  <div style={label}>Objective</div>
                  <div style={{ ...value, lineHeight: 1.5 }}>
                    {mission.objective.length > 200
                      ? mission.objective.slice(0, 200) + "..."
                      : mission.objective}
                  </div>
                </div>
                {/* Stats in a row — only short values */}
                <div
                  style={{
                    display: "flex",
                    gap: 32,
                    borderTop: "1px solid var(--border-subtle, #e0e0e0)",
                    paddingTop: 16,
                  }}
                >
                  <div>
                    <div style={label}>Elapsed</div>
                    <div style={value}>{formatElapsed(mission.createdAt)}</div>
                  </div>
                  <div>
                    <div style={label}>Phase</div>
                    <div style={value}>
                      {missionPhaseLabels[mission.phase] ?? mission.phase}
                    </div>
                  </div>
                  <div>
                    <div style={label}>Budget</div>
                    <div style={value}>
                      {mission.budget ?? mission.estimatedCost ?? "N/A"}
                    </div>
                  </div>
                </div>
              </div>
            ) : (
              <div style={{ ...card, textAlign: "center", padding: 48 }}>
                <p
                  style={{
                    fontSize: 14,
                    color: "var(--text-secondary, #525252)",
                  }}
                >
                  No mission data found.
                </p>
              </div>
            )}

            {/* Research progress */}
            {kb && (
              <div style={card} className={navStatus === "Running" ? "animate-breathe" : ""}>
                <div style={sectionTitle}>Research Progress</div>

                {/* Artifact counts */}
                <div
                  style={{
                    display: "flex",
                    gap: 32,
                    marginBottom: 24,
                  }}
                >
                  {[
                    { count: kb.artifactCounts.hypotheses, label: "Hypotheses" },
                    { count: kb.artifactCounts.experiments, label: "Experiments" },
                    { count: kb.artifactCounts.findings, label: "Findings" },
                    { count: kb.artifactCounts.challengeReviews, label: "Reviews" },
                  ].map((item) => (
                    <div key={item.label} style={{ textAlign: "center" }}>
                      <div
                        style={{
                          fontSize: 24,
                          fontWeight: 600,
                          color: "var(--text-primary, #161616)",
                        }}
                      >
                        {item.count}
                      </div>
                      <div
                        style={{
                          fontSize: 12,
                          color: "var(--text-secondary, #525252)",
                          marginTop: 4,
                        }}
                      >
                        {item.label}
                      </div>
                    </div>
                  ))}
                </div>

                {/* Phase bar */}
                <div style={{ marginBottom: 8 }}>
                  <div style={label}>Current Activity</div>
                  <div
                    style={{
                      display: "flex",
                      gap: 4,
                      marginTop: 8,
                    }}
                  >
                    {phaseSteps.map((step, i) => (
                      <div
                        key={step}
                        className={i === phaseIndex && navStatus === "Running" ? "animate-shimmer" : ""}
                        style={{
                          flex: 1,
                          height: 4,
                          borderRadius: 2,
                          backgroundColor:
                            i <= phaseIndex
                              ? "var(--interactive, #0f62fe)"
                              : "#e0e0e0",
                          transition: "background-color 300ms",
                        }}
                      />
                    ))}
                  </div>
                  <div
                    style={{
                      fontSize: 12,
                      color: "var(--text-secondary, #525252)",
                      marginTop: 8,
                    }}
                  >
                    {phaseLabels[kb.phase] ?? kb.phase}
                  </div>
                </div>
              </div>
            )}

            {/* Empty state: no artifacts yet but mission is running */}
            {mission &&
              navStatus === "Running" &&
              totalArtifacts === 0 &&
              kb && (
                <div style={{ ...card, textAlign: "center", padding: 48 }}>
                  <div
                    style={{
                      display: "inline-flex",
                      alignItems: "center",
                      gap: 8,
                      marginBottom: 16,
                    }}
                  >
                    <span
                      style={{
                        display: "inline-block",
                        width: 8,
                        height: 8,
                        borderRadius: "50%",
                        backgroundColor: "var(--interactive, #0f62fe)",
                        animation: "pulse 2s ease-in-out infinite",
                      }}
                    />
                    <span
                      style={{
                        fontSize: 14,
                        fontWeight: 500,
                        color: "var(--text-primary, #161616)",
                      }}
                    >
                      Research is thinking...
                    </span>
                  </div>
                  <p
                    style={{
                      fontSize: 13,
                      color: "var(--text-secondary, #525252)",
                    }}
                  >
                    Research is starting up. First artifacts typically appear
                    within 15-30 minutes.
                  </p>
                </div>
              )}

            {/* Latest activity feed */}
            {recentArtifacts.length > 0 && (
              <div style={card}>
                <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
                  <div style={sectionTitle}>Latest Activity</div>
                  {navStatus === "Running" && (
                    <span style={{ fontSize: 11, color: "#525252" }} className="animate-pulse-dot">
                      updating...
                    </span>
                  )}
                </div>
                <div>
                  {recentArtifacts.map((artifact, idx) => (
                    <Link
                      href={`/artifact/${artifact.id}`}
                      key={artifact.id}
                      className="animate-fade-in"
                      style={{
                        display: "flex",
                        alignItems: "center",
                        gap: 12,
                        padding: "10px 0",
                        borderBottom: "1px solid #e0e0e0",
                        animationDelay: `${idx * 50}ms`,
                        textDecoration: "none",
                        cursor: "pointer",
                      }}
                    >
                      <span
                        style={{
                          fontSize: 11,
                          fontWeight: 600,
                          fontFamily: "'IBM Plex Mono', monospace",
                          color:
                            artifactTypeColors[artifact.type] ?? "#525252",
                          minWidth: 40,
                        }}
                      >
                        {artifact.id}
                      </span>
                      <span
                        style={{
                          fontSize: 11,
                          fontWeight: 500,
                          color: "#ffffff",
                          backgroundColor:
                            artifactTypeColors[artifact.type] ?? "#525252",
                          padding: "1px 6px",
                          borderRadius: 4,
                          minWidth: 64,
                          textAlign: "center",
                        }}
                      >
                        {artifactTypeLabels[artifact.type] ?? artifact.type}
                      </span>
                      <span
                        style={{
                          flex: 1,
                          fontSize: 13,
                          color: "var(--text-primary, #161616)",
                        }}
                      >
                        {artifact.title}
                      </span>
                      <span
                        style={{
                          fontSize: 12,
                          color: "var(--text-secondary, #525252)",
                        }}
                      >
                        {artifact.status}
                      </span>
                    </Link>
                  ))}
                </div>
              </div>
            )}
          </>
        )}
      </main>

      <style>{`
        @keyframes pulse {
          0%, 100% { opacity: 1; }
          50% { opacity: 0.4; }
        }
      `}</style>
    </div>
  );
}
