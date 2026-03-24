"use client";

import { useEffect, useState, useCallback } from "react";
import Nav from "../../components/nav";

interface CeoRequest {
  id: string;
  title: string;
  status: string;
  description: string;
  date?: string;
}

interface Artifact {
  type: string;
  id: string;
  title: string;
  status: string;
  content: string;
  metadata: Record<string, string>;
}

interface StatusResponse {
  mission: {
    id: string;
    phase: string;
    objective: string;
  } | null;
  kb: {
    artifacts: Artifact[];
    ceoRequests: CeoRequest[];
    artifactCounts: {
      hypotheses: number;
      experiments: number;
      findings: number;
      challengeReviews: number;
      tasks: number;
    };
  } | null;
  pendingEscalations: CeoRequest[];
}

interface ChallengeReviewSummary {
  id: string;
  title: string;
  content: string;
  counts: { critical: number; high: number; medium: number; low: number };
}

function parseSeverityCounts(content: string): {
  critical: number;
  high: number;
  medium: number;
  low: number;
} {
  const counts = { critical: 0, high: 0, medium: 0, low: 0 };
  const critMatch = content.match(/critical/gi);
  const highMatch = content.match(/\bhigh\b/gi);
  const medMatch = content.match(/\bmedium\b/gi);
  const lowMatch = content.match(/\blow\b/gi);
  if (critMatch) counts.critical = critMatch.length;
  if (highMatch) counts.high = highMatch.length;
  if (medMatch) counts.medium = medMatch.length;
  if (lowMatch) counts.low = lowMatch.length;
  return counts;
}

export default function SteeringPage() {
  const [status, setStatus] = useState<StatusResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [responses, setResponses] = useState<Record<string, string>>({});
  const [respondingTo, setRespondingTo] = useState<string | null>(null);
  const [showChallengeReviews, setShowChallengeReviews] = useState(false);
  const [feedbackArtifact, setFeedbackArtifact] = useState("");
  const [feedbackComment, setFeedbackComment] = useState("");
  const [feedbackSubmitting, setFeedbackSubmitting] = useState(false);
  const [feedbackSuccess, setFeedbackSuccess] = useState(false);

  const fetchStatus = useCallback(async () => {
    try {
      const res = await fetch("/api/status");
      if (!res.ok) throw new Error("Failed to load status");
      const data: StatusResponse = await res.json();
      setStatus(data);
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

  async function handleRespond(requestId: string) {
    const response = responses[requestId];
    if (!response?.trim()) return;
    setRespondingTo(requestId);
    try {
      await fetch("/api/escalation", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ requestId, response }),
      });
      setResponses((prev) => {
        const next = { ...prev };
        delete next[requestId];
        return next;
      });
    } catch {
      // Next poll will correct
    }
    setRespondingTo(null);
  }

  async function handleFeedbackSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!feedbackArtifact || !feedbackComment.trim()) return;
    setFeedbackSubmitting(true);
    try {
      const res = await fetch("/api/feedback", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          artifactId: feedbackArtifact,
          comment: feedbackComment,
        }),
      });
      if (res.ok) {
        setFeedbackComment("");
        setFeedbackArtifact("");
        setFeedbackSuccess(true);
        setTimeout(() => setFeedbackSuccess(false), 3000);
      }
    } catch {
      // Silently handle
    }
    setFeedbackSubmitting(false);
  }

  if (loading) {
    return (
      <div
        style={{
          minHeight: "100vh", marginLeft: 64,
          backgroundColor: "#f4f4f4",
          fontFamily: "'IBM Plex Sans', sans-serif",
        }}
      >
        <Nav activePath="/steering" />
        <div style={{ maxWidth: 960, margin: "0 auto", padding: 24 }}>
          <div
            style={{
              backgroundColor: "#ffffff",
              border: "1px solid #e0e0e0",
              padding: 48,
              textAlign: "center",
              fontSize: 14,
              color: "#525252",
            }}
          >
            Loading steering data...
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
        <Nav activePath="/steering" />
        <div style={{ maxWidth: 960, margin: "0 auto", padding: 24 }}>
          <div
            style={{
              backgroundColor: "#ffffff",
              border: "1px solid #e0e0e0",
              padding: 48,
              textAlign: "center",
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

  const kb = status?.kb;
  const pendingRequests =
    kb?.ceoRequests.filter((r) => r.status === "PENDING") ?? [];

  const challengeReviews: ChallengeReviewSummary[] =
    kb?.artifacts
      .filter((a) => a.type === "challenge_review")
      .map((a) => ({
        id: a.id,
        title: a.title,
        content: a.content,
        counts: parseSeverityCounts(a.content),
      })) ?? [];

  const allArtifacts =
    kb?.artifacts.filter((a) =>
      ["hypothesis", "experiment", "finding"].includes(a.type)
    ) ?? [];

  // Style constants
  const card: React.CSSProperties = {
    backgroundColor: "#ffffff",
    border: "1px solid #e0e0e0",
    borderRadius: 0,
    padding: 24,
    fontFamily: "'IBM Plex Sans', sans-serif",
  };

  const sectionTitle = (color: string = "#525252"): React.CSSProperties => ({
    fontSize: 12,
    fontWeight: 600,
    textTransform: "uppercase",
    letterSpacing: "0.04em",
    color,
    marginBottom: 16,
  });

  return (
    <div
      style={{
        minHeight: "100vh", marginLeft: 64,
        backgroundColor: "#f4f4f4",
        fontFamily: "'IBM Plex Sans', sans-serif",
      }}
    >
      <Nav activePath="/steering" />

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
          Steering Controls
        </h1>

        {/* CEO Requests Section */}
        <div>
          <div style={sectionTitle()}>CEO Requests</div>
          {pendingRequests.length === 0 ? (
            <div style={{ ...card, textAlign: "center", padding: 48 }}>
              <p style={{ fontSize: 14, color: "#525252" }}>
                No pending requests. The agent has not escalated anything yet.
              </p>
            </div>
          ) : (
            <div
              style={{ display: "flex", flexDirection: "column", gap: 12 }}
            >
              {pendingRequests.map((req) => (
                <div
                  key={req.id}
                  style={{
                    ...card,
                    borderColor: "#da1e28",
                    borderWidth: 2,
                    backgroundColor: "#fff1f1",
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
                        color: "var(--text-secondary, #525252)",
                      }}
                    >
                      {req.id}
                    </span>
                    <span
                      style={{
                        fontSize: 14,
                        fontWeight: 600,
                        color: "#161616",
                      }}
                    >
                      {req.title}
                    </span>
                  </div>
                  <p style={{ fontSize: 13, color: "#525252", marginBottom: 4 }}>
                    {req.description}
                  </p>
                  {req.date && (
                    <p
                      style={{
                        fontSize: 12,
                        color: "#8d8d8d",
                        marginBottom: 8,
                      }}
                    >
                      Requested: {req.date}
                    </p>
                  )}
                  <p
                    style={{
                      fontSize: 12,
                      fontWeight: 500,
                      color: "var(--text-secondary, #525252)",
                      marginBottom: 12,
                    }}
                  >
                    Why needed: The agent is blocked and cannot proceed without
                    your input.
                  </p>
                  <div style={{ display: "flex", gap: 8 }}>
                    <textarea
                      value={responses[req.id] || ""}
                      onChange={(e) =>
                        setResponses((prev) => ({
                          ...prev,
                          [req.id]: e.target.value,
                        }))
                      }
                      placeholder="Your response..."
                      rows={2}
                      style={{
                        flex: 1,
                        padding: "8px 12px",
                        border: "1px solid #e0e0e0",
                        fontSize: 14,
                        fontFamily: "'IBM Plex Sans', sans-serif",
                        backgroundColor: "#ffffff",
                        resize: "vertical",
                      }}
                    />
                    <button
                      onClick={() => handleRespond(req.id)}
                      disabled={
                        respondingTo === req.id ||
                        !responses[req.id]?.trim()
                      }
                      style={{
                        padding: "8px 16px",
                        backgroundColor:
                          respondingTo === req.id ||
                          !responses[req.id]?.trim()
                            ? "#c6c6c6"
                            : "#da1e28",
                        color: "#ffffff",
                        border: "none",
                        fontSize: 14,
                        fontWeight: 500,
                        fontFamily: "'IBM Plex Sans', sans-serif",
                        cursor:
                          respondingTo === req.id ||
                          !responses[req.id]?.trim()
                            ? "not-allowed"
                            : "pointer",
                        alignSelf: "flex-end",
                      }}
                    >
                      {respondingTo === req.id ? "Sending..." : "Respond"}
                    </button>
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>

        {/* Challenge Reviews Section */}
        <div>
          <div
            style={{
              display: "flex",
              alignItems: "center",
              justifyContent: "space-between",
              marginBottom: 16,
            }}
          >
            <div style={sectionTitle()}>Challenge Reviews</div>
            {challengeReviews.length > 0 && (
              <button
                onClick={() => setShowChallengeReviews(!showChallengeReviews)}
                style={{
                  fontSize: 13,
                  color: "var(--text-secondary, #525252)",
                  background: "none",
                  border: "none",
                  cursor: "pointer",
                  fontWeight: 500,
                  fontFamily: "'IBM Plex Sans', sans-serif",
                }}
              >
                {showChallengeReviews ? "Hide reviews" : "View past reviews"}
              </button>
            )}
          </div>
          {challengeReviews.length === 0 ? (
            <div style={{ ...card, textAlign: "center", padding: 48 }}>
              <p style={{ fontSize: 14, color: "#525252" }}>
                No challenge reviews yet. Reviews appear when the agent
                evaluates its own findings.
              </p>
            </div>
          ) : (
            showChallengeReviews && (
              <div
                style={{ display: "flex", flexDirection: "column", gap: 8 }}
              >
                {challengeReviews.map((cr) => (
                  <div key={cr.id} style={card}>
                    <div
                      style={{
                        display: "flex",
                        alignItems: "center",
                        gap: 8,
                        marginBottom: 12,
                      }}
                    >
                      <span
                        style={{
                          fontFamily: "'IBM Plex Mono', monospace",
                          fontSize: 12,
                          fontWeight: 600,
                          color: "var(--text-secondary, #525252)",
                        }}
                      >
                        {cr.id}
                      </span>
                      <span
                        style={{ fontSize: 14, fontWeight: 500, color: "#161616" }}
                      >
                        {cr.title}
                      </span>
                    </div>
                    <div style={{ display: "flex", gap: 8 }}>
                      {cr.counts.critical > 0 && (
                        <span
                          style={{
                            fontSize: 11,
                            fontWeight: 500,
                            color: "#ffffff",
                            backgroundColor: "#da1e28",
                            padding: "1px 8px",
                            borderRadius: 2,
                          }}
                        >
                          {cr.counts.critical} critical
                        </span>
                      )}
                      {cr.counts.high > 0 && (
                        <span
                          style={{
                            fontSize: 11,
                            fontWeight: 500,
                            color: "#ffffff",
                            backgroundColor: "#ff832b",
                            padding: "1px 8px",
                            borderRadius: 2,
                          }}
                        >
                          {cr.counts.high} high
                        </span>
                      )}
                      {cr.counts.medium > 0 && (
                        <span
                          style={{
                            fontSize: 11,
                            fontWeight: 500,
                            color: "#161616",
                            backgroundColor: "#f1c21b",
                            padding: "1px 8px",
                            borderRadius: 2,
                          }}
                        >
                          {cr.counts.medium} medium
                        </span>
                      )}
                      {cr.counts.low > 0 && (
                        <span
                          style={{
                            fontSize: 11,
                            fontWeight: 500,
                            color: "#525252",
                            backgroundColor: "#e0e0e0",
                            padding: "1px 8px",
                            borderRadius: 2,
                          }}
                        >
                          {cr.counts.low} low
                        </span>
                      )}
                    </div>
                  </div>
                ))}
              </div>
            )
          )}
        </div>

        {/* Feedback Section */}
        <div>
          <div style={sectionTitle("#525252")}>Feedback</div>
          {allArtifacts.length === 0 ? (
            <div style={{ ...card, textAlign: "center", padding: 48 }}>
              <p style={{ fontSize: 14, color: "#525252" }}>
                No artifacts to provide feedback on yet. Feedback will be
                available once the agent produces hypotheses or findings.
              </p>
            </div>
          ) : (
            <div style={card}>
              <form
                onSubmit={handleFeedbackSubmit}
                style={{
                  display: "flex",
                  flexDirection: "column",
                  gap: 16,
                }}
              >
                <div>
                  <label
                    htmlFor="artifact-select"
                    style={{
                      display: "block",
                      fontSize: 12,
                      color: "#525252",
                      marginBottom: 4,
                    }}
                  >
                    Select artifact
                  </label>
                  <select
                    id="artifact-select"
                    value={feedbackArtifact}
                    onChange={(e) => setFeedbackArtifact(e.target.value)}
                    required
                    style={{
                      width: "100%",
                      padding: "8px 12px",
                      border: "1px solid #e0e0e0",
                      fontSize: 14,
                      fontFamily: "'IBM Plex Sans', sans-serif",
                      backgroundColor: "#ffffff",
                      color: "#161616",
                      appearance: "auto",
                    }}
                  >
                    <option value="">Choose an artifact...</option>
                    {allArtifacts.map((a) => (
                      <option key={a.id} value={a.id}>
                        {a.id} - {a.title}
                      </option>
                    ))}
                  </select>
                </div>
                <div>
                  <label
                    htmlFor="feedback-comment"
                    style={{
                      display: "block",
                      fontSize: 12,
                      color: "#525252",
                      marginBottom: 4,
                    }}
                  >
                    Comment
                  </label>
                  <textarea
                    id="feedback-comment"
                    value={feedbackComment}
                    onChange={(e) => setFeedbackComment(e.target.value)}
                    placeholder="Your feedback on this artifact..."
                    rows={3}
                    required
                    style={{
                      width: "100%",
                      padding: "8px 12px",
                      border: "1px solid #e0e0e0",
                      fontSize: 14,
                      fontFamily: "'IBM Plex Sans', sans-serif",
                      backgroundColor: "#ffffff",
                      color: "#161616",
                      resize: "vertical",
                    }}
                  />
                </div>
                <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
                  <button
                    type="submit"
                    disabled={
                      feedbackSubmitting ||
                      !feedbackArtifact ||
                      !feedbackComment.trim()
                    }
                    style={{
                      padding: "8px 16px",
                      backgroundColor:
                        feedbackSubmitting ||
                        !feedbackArtifact ||
                        !feedbackComment.trim()
                          ? "#c6c6c6"
                          : "#161616",
                      color: "#ffffff",
                      border: "none",
                      fontSize: 14,
                      fontWeight: 500,
                      fontFamily: "'IBM Plex Sans', sans-serif",
                      cursor:
                        feedbackSubmitting ||
                        !feedbackArtifact ||
                        !feedbackComment.trim()
                          ? "not-allowed"
                          : "pointer",
                    }}
                  >
                    {feedbackSubmitting ? "Submitting..." : "Submit"}
                  </button>
                  {feedbackSuccess && (
                    <span
                      style={{
                        fontSize: 12,
                        fontWeight: 500,
                        color: "#198038",
                      }}
                    >
                      Feedback submitted
                    </span>
                  )}
                </div>
              </form>
            </div>
          )}
        </div>
      </main>
    </div>
  );
}
