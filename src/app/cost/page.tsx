"use client";

import { useEffect, useState, useCallback } from "react";
import Nav from "@/components/nav";

interface MissionData {
  id: string;
  phase: string;
  objective: string;
  createdAt: string;
  completedAt?: string;
  estimatedCost?: string;
}

interface StatusResponse {
  mission: MissionData | null;
  kb: {
    artifacts: Array<{ type: string }>;
    artifactCounts: {
      hypotheses: number;
      experiments: number;
      findings: number;
      challengeReviews: number;
      tasks: number;
    };
  } | null;
}

interface CostData {
  budget: number | null;
  spent: number;
  phases: Array<{ name: string; startedAt: string; endedAt?: string }>;
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

function formatElapsed(fromISO: string, toISO?: string): string {
  return formatDuration(fromISO, toISO);
}

export default function CostPage() {
  const [status, setStatus] = useState<StatusResponse | null>(null);
  const [costData, setCostData] = useState<CostData | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [budgetInput, setBudgetInput] = useState("");
  const [budgetSaving, setBudgetSaving] = useState(false);
  const [budgetSaved, setBudgetSaved] = useState(false);
  const [budgetInitialized, setBudgetInitialized] = useState(false);

  const fetchStatus = useCallback(async () => {
    try {
      const [statusRes, costRes] = await Promise.all([
        fetch("/api/status"),
        fetch("/api/cost"),
      ]);
      if (!statusRes.ok) throw new Error("Failed to load status");
      const statusData: StatusResponse = await statusRes.json();
      setStatus(statusData);

      if (costRes.ok) {
        const cd: CostData = await costRes.json();
        setCostData(cd);
        if (cd.budget !== null && !budgetInitialized) {
          setBudgetInput(String(cd.budget));
          setBudgetInitialized(true);
        }
      }

      setError(null);
    } catch (err) {
      if (loading) {
        setError(err instanceof Error ? err.message : "Failed to load");
      }
    } finally {
      setLoading(false);
    }
  }, [loading, budgetInitialized]);

  useEffect(() => {
    fetchStatus();
    const interval = setInterval(fetchStatus, 5000);
    return () => clearInterval(interval);
  }, [fetchStatus]);

  async function handleBudgetSave() {
    const budget = parseFloat(budgetInput);
    if (isNaN(budget) || budget < 0) return;
    setBudgetSaving(true);
    try {
      await fetch("/api/cost", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ budget }),
      });
      setBudgetSaved(true);
      setBudgetInitialized(true);
      setTimeout(() => setBudgetSaved(false), 3000);
    } catch {
      // Silently handle
    }
    setBudgetSaving(false);
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
        <Nav activePath="/cost" />
        <div style={{ maxWidth: 960, margin: "0 auto", padding: 24 }}>
          <div style={{ ...card, textAlign: "center", padding: 48, color: "#525252", fontSize: 14 }}>
            Loading cost data...
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
        <Nav activePath="/cost" />
        <div style={{ maxWidth: 960, margin: "0 auto", padding: 24 }}>
          <div style={{ ...card, textAlign: "center", padding: 48, color: "#525252", fontSize: 14 }}>
            {error}
          </div>
        </div>
      </div>
    );
  }

  const mission = status?.mission;
  const kb = status?.kb;

  // Empty state: no mission data
  if (!mission) {
    return (
      <div
        style={{
          minHeight: "100vh", marginLeft: 64,
          backgroundColor: "#f4f4f4",
          fontFamily: "'IBM Plex Sans', sans-serif",
        }}
      >
        <Nav activePath="/cost" />
        <div style={{ maxWidth: 960, margin: "0 auto", padding: 24 }}>
          <h1 style={{ fontSize: 20, fontWeight: 600, color: "#161616", marginBottom: 24 }}>
            Cost Tracker
          </h1>
          <div style={{ ...card, textAlign: "center", padding: 48 }}>
            <p style={{ fontSize: 14, color: "#525252" }}>
              Cost tracking begins when agent starts processing.
            </p>
          </div>
        </div>
      </div>
    );
  }

  const findingsCount = kb?.artifactCounts.findings ?? 0;
  const spent = costData?.spent ?? 0;
  const budget = costData?.budget ?? null;
  const budgetPct =
    budget !== null && budget > 0
      ? Math.min((spent / budget) * 100, 100)
      : 0;
  const costPerFinding =
    findingsCount > 0 ? (spent / findingsCount).toFixed(2) : null;

  return (
    <div
      style={{
        minHeight: "100vh", marginLeft: 64,
        backgroundColor: "#f4f4f4",
        fontFamily: "'IBM Plex Sans', sans-serif",
      }}
    >
      <Nav activePath="/cost" />

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
        <h1 style={{ fontSize: 20, fontWeight: 600, color: "#161616", marginBottom: 0 }}>
          Cost Tracker
        </h1>

        {/* Elapsed Time */}
        <div style={card}>
          <div style={sectionLabel}>Elapsed Time</div>
          <div
            style={{
              fontSize: 32,
              fontWeight: 600,
              fontFamily: "'IBM Plex Mono', monospace",
              color: "#161616",
              letterSpacing: "-0.02em",
            }}
          >
            {formatDuration(mission.createdAt, mission.completedAt)}
          </div>
          {!["COMPLETED", "KILLED"].includes(mission.phase) && (
            <div style={{ fontSize: 12, color: "#8d8d8d", marginTop: 4 }}>
              Running since {new Date(mission.createdAt).toLocaleString()}
            </div>
          )}
        </div>

        {/* Budget Gauge */}
        <div style={card}>
          <div style={sectionLabel}>Budget</div>
          {budget !== null ? (
            <>
              <div
                style={{
                  display: "flex",
                  justifyContent: "space-between",
                  fontSize: 14,
                  marginBottom: 8,
                }}
              >
                <span style={{ fontWeight: 500, color: "#161616" }}>
                  ${spent.toFixed(2)} spent
                </span>
                <span style={{ color: "#525252" }}>
                  ${budget.toFixed(2)} budget
                </span>
              </div>
              <div
                style={{
                  width: "100%",
                  height: 8,
                  backgroundColor: "#e0e0e0",
                  overflow: "hidden",
                }}
              >
                <div
                  style={{
                    height: "100%",
                    width: `${budgetPct}%`,
                    backgroundColor:
                      budgetPct > 90
                        ? "#da1e28"
                        : budgetPct > 70
                          ? "#f1c21b"
                          : "#198038",
                    transition: "width 0.5s ease",
                  }}
                />
              </div>
              <div style={{ fontSize: 12, color: "#8d8d8d", marginTop: 4 }}>
                {budgetPct.toFixed(1)}% used
              </div>
            </>
          ) : (
            <p style={{ fontSize: 14, color: "#525252" }}>
              No budget set. Use the editor below to set one.
            </p>
          )}
        </div>

        {/* Budget Editor */}
        <div style={card}>
          <div style={sectionLabel}>Update Budget</div>
          <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
            <div style={{ position: "relative", flex: 1 }}>
              <span
                style={{
                  position: "absolute",
                  left: 12,
                  top: "50%",
                  transform: "translateY(-50%)",
                  fontSize: 14,
                  color: "#525252",
                }}
              >
                $
              </span>
              <input
                type="number"
                min="0"
                step="0.01"
                value={budgetInput}
                onChange={(e) => setBudgetInput(e.target.value)}
                placeholder="100.00"
                style={{
                  width: "100%",
                  padding: "8px 12px 8px 24px",
                  border: "1px solid #e0e0e0",
                  fontSize: 14,
                  fontFamily: "'IBM Plex Sans', sans-serif",
                  backgroundColor: "#ffffff",
                  color: "#161616",
                }}
              />
            </div>
            <button
              onClick={handleBudgetSave}
              disabled={budgetSaving || !budgetInput}
              style={{
                padding: "8px 16px",
                backgroundColor:
                  budgetSaving || !budgetInput ? "#c6c6c6" : "#161616",
                color: "#ffffff",
                border: "none",
                fontSize: 14,
                fontWeight: 500,
                fontFamily: "'IBM Plex Sans', sans-serif",
                cursor:
                  budgetSaving || !budgetInput ? "not-allowed" : "pointer",
              }}
            >
              {budgetSaving ? "Saving..." : "Save"}
            </button>
            {budgetSaved && (
              <span style={{ fontSize: 12, fontWeight: 500, color: "#198038" }}>
                Saved
              </span>
            )}
          </div>
        </div>

        {/* Cost Per Finding */}
        <div style={card}>
          <div style={sectionLabel}>Cost Per Finding</div>
          {costPerFinding ? (
            <div
              style={{
                fontSize: 28,
                fontWeight: 600,
                fontFamily: "'IBM Plex Mono', monospace",
                color: "#161616",
              }}
            >
              ${costPerFinding}
            </div>
          ) : (
            <p style={{ fontSize: 14, color: "#525252" }}>
              No findings yet. This metric will appear once the agent produces
              findings.
            </p>
          )}
          <div style={{ fontSize: 12, color: "#8d8d8d", marginTop: 4 }}>
            {findingsCount} finding{findingsCount !== 1 ? "s" : ""} total
          </div>
        </div>

        {/* Phase Duration Breakdown */}
        <div style={card}>
          <div style={sectionLabel}>Phase Duration Breakdown</div>
          {costData?.phases && costData.phases.length > 0 ? (
            <div>
              {costData.phases.map((phase, i) => (
                <div
                  key={i}
                  style={{
                    display: "flex",
                    justifyContent: "space-between",
                    alignItems: "center",
                    padding: "10px 0",
                    borderBottom:
                      i < costData.phases.length - 1
                        ? "1px solid #e0e0e0"
                        : "none",
                  }}
                >
                  <span
                    style={{
                      fontSize: 14,
                      fontWeight: 500,
                      color: "#161616",
                      textTransform: "capitalize",
                    }}
                  >
                    {phase.name.replace(/_/g, " ")}
                  </span>
                  <span
                    style={{
                      fontSize: 14,
                      fontFamily: "'IBM Plex Mono', monospace",
                      color: "#525252",
                    }}
                  >
                    {formatElapsed(phase.startedAt, phase.endedAt)}
                  </span>
                </div>
              ))}
            </div>
          ) : (
            <p style={{ fontSize: 14, color: "#525252" }}>
              Phase timing data will appear as the mission progresses.
            </p>
          )}
        </div>
      </main>
    </div>
  );
}
