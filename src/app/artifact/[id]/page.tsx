"use client";

import { useEffect, useState, use } from "react";
import Link from "next/link";
import Markdown from "react-markdown";
import Nav from "../../../components/nav";
import StatusBadge from "../../../components/status-badge";

interface Artifact {
  type: string;
  id: string;
  title: string;
  status: string;
  metadata: Record<string, string>;
  content: string;
  filePath: string;
}

const typeLabels: Record<string, string> = {
  hypothesis: "Hypothesis",
  experiment: "Experiment",
  finding: "Finding",
  challenge_review: "Challenge Review",
  strategic_review: "Strategic Review",
  report: "Report",
  task: "Task",
  literature: "Literature",
};

const typeBorderColors: Record<string, string> = {
  hypothesis: "#0f62fe",
  experiment: "#f1c21b",
  finding: "#198038",
  challenge_review: "#8a3ffc",
  strategic_review: "#8a3ffc",
  report: "#8a3ffc",
  task: "#525252",
  literature: "#005d5d",
};

export default function ArtifactDetail({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = use(params);
  const [artifact, setArtifact] = useState<Artifact | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    fetch(`/api/artifact/${id}`)
      .then((r) => {
        if (!r.ok) throw new Error("Not found");
        return r.json();
      })
      .then((data) => {
        setArtifact(data.artifact);
        setLoading(false);
      })
      .catch((err) => {
        setError(err.message);
        setLoading(false);
      });
  }, [id]);

  const borderColor = artifact
    ? typeBorderColors[artifact.type] || "#525252"
    : "#e0e0e0";

  return (
    <div
      style={{
        minHeight: "100vh", marginLeft: 64,
        backgroundColor: "#f4f4f4",
        fontFamily: "'IBM Plex Sans', sans-serif",
      }}
    >
      <Nav activePath="/research" />

      <main
        style={{
          maxWidth: 900,
          margin: "0 auto",
          padding: "24px 24px 64px",
        }}
      >
        {/* Back link */}
        <Link
          href="/research"
          style={{
            fontSize: 13,
            color: "var(--interactive, #0f62fe)",
            textDecoration: "none",
            display: "inline-flex",
            alignItems: "center",
            gap: 4,
            marginBottom: 16,
          }}
        >
          ← Back to Research
        </Link>

        {loading && (
          <div
            style={{
              backgroundColor: "#ffffff",
              border: "1px solid #e0e0e0",
              borderRadius: 8,
              padding: 48,
              textAlign: "center",
            }}
          >
            <p style={{ color: "#525252", fontSize: 14 }}>
              Loading artifact...
            </p>
          </div>
        )}

        {error && (
          <div
            style={{
              backgroundColor: "#ffffff",
              border: "1px solid #e0e0e0",
              borderRadius: 8,
              padding: 48,
              textAlign: "center",
            }}
          >
            <p style={{ color: "#da1e28", fontSize: 14 }}>
              Artifact not found: {id}
            </p>
            <Link
              href="/research"
              style={{
                fontSize: 13,
                color: "#0f62fe",
                textDecoration: "underline",
                marginTop: 8,
                display: "inline-block",
              }}
            >
              Return to Research
            </Link>
          </div>
        )}

        {artifact && (
          <div
            style={{
              backgroundColor: "#ffffff",
              border: "1px solid #e0e0e0",
              borderLeft: `4px solid ${borderColor}`,
              borderRadius: 8,
              overflow: "hidden",
            }}
          >
            {/* Header */}
            <div
              style={{
                padding: "24px 32px",
                borderBottom: "1px solid #e0e0e0",
                display: "flex",
                alignItems: "flex-start",
                justifyContent: "space-between",
                gap: 16,
              }}
            >
              <div>
                <div
                  style={{
                    display: "flex",
                    alignItems: "center",
                    gap: 12,
                    marginBottom: 8,
                  }}
                >
                  <span
                    style={{
                      fontSize: 12,
                      fontWeight: 600,
                      fontFamily: "'IBM Plex Mono', monospace",
                      color: borderColor,
                    }}
                  >
                    {artifact.id}
                  </span>
                  <span
                    style={{
                      fontSize: 11,
                      color: "#525252",
                      backgroundColor: "#f4f4f4",
                      padding: "2px 8px",
                      borderRadius: 4,
                    }}
                  >
                    {typeLabels[artifact.type] || artifact.type}
                  </span>
                </div>
                <h1
                  style={{
                    fontSize: 20,
                    fontWeight: 600,
                    color: "#161616",
                    margin: 0,
                    lineHeight: 1.4,
                  }}
                >
                  {artifact.title}
                </h1>
              </div>
              <StatusBadge status={artifact.status} size="md" />
            </div>

            {/* Metadata */}
            {Object.keys(artifact.metadata).length > 0 && (
              <div
                style={{
                  padding: "16px 32px",
                  borderBottom: "1px solid #e0e0e0",
                  backgroundColor: "#fafafa",
                  display: "flex",
                  flexWrap: "wrap",
                  gap: 24,
                }}
              >
                {Object.entries(artifact.metadata).map(([key, val]) => (
                  <div key={key}>
                    <span
                      style={{
                        fontSize: 11,
                        fontWeight: 600,
                        textTransform: "uppercase",
                        letterSpacing: "0.04em",
                        color: "#525252",
                      }}
                    >
                      {key}
                    </span>
                    <div
                      style={{
                        fontSize: 13,
                        color: "#161616",
                        marginTop: 2,
                      }}
                    >
                      {val}
                    </div>
                  </div>
                ))}
              </div>
            )}

            {/* Markdown content */}
            <div
              style={{
                padding: "32px",
              }}
              className="artifact-content"
            >
              <Markdown>{artifact.content}</Markdown>
            </div>
          </div>
        )}
      </main>

      <style>{`
        .artifact-content h1 {
          font-size: 20px;
          font-weight: 600;
          color: #161616;
          margin: 24px 0 12px;
          padding-bottom: 8px;
          border-bottom: 1px solid #e0e0e0;
        }
        .artifact-content h2 {
          font-size: 16px;
          font-weight: 600;
          color: #161616;
          margin: 20px 0 8px;
        }
        .artifact-content h3 {
          font-size: 14px;
          font-weight: 600;
          color: #161616;
          margin: 16px 0 8px;
        }
        .artifact-content p {
          font-size: 14px;
          line-height: 1.6;
          color: #161616;
          margin: 0 0 12px;
        }
        .artifact-content ul, .artifact-content ol {
          font-size: 14px;
          line-height: 1.6;
          color: #161616;
          padding-left: 24px;
          margin: 0 0 12px;
        }
        .artifact-content li {
          margin-bottom: 4px;
        }
        .artifact-content code {
          font-family: 'IBM Plex Mono', monospace;
          font-size: 13px;
          background-color: #f4f4f4;
          padding: 2px 6px;
          border-radius: 3px;
        }
        .artifact-content pre {
          font-family: 'IBM Plex Mono', monospace;
          font-size: 13px;
          background-color: #f4f4f4;
          padding: 16px;
          border-radius: 4px;
          overflow-x: auto;
          margin: 0 0 16px;
        }
        .artifact-content pre code {
          background: none;
          padding: 0;
        }
        .artifact-content table {
          width: 100%;
          border-collapse: collapse;
          font-size: 13px;
          margin: 0 0 16px;
        }
        .artifact-content th {
          text-align: left;
          font-weight: 600;
          padding: 8px 12px;
          border-bottom: 2px solid #e0e0e0;
          color: #161616;
        }
        .artifact-content td {
          padding: 8px 12px;
          border-bottom: 1px solid #e0e0e0;
          color: #161616;
        }
        .artifact-content blockquote {
          border-left: 3px solid #e0e0e0;
          padding: 4px 16px;
          margin: 0 0 12px;
          color: #525252;
          font-size: 13px;
        }
        .artifact-content strong {
          font-weight: 600;
        }
        .artifact-content a {
          color: #0f62fe;
          text-decoration: none;
        }
        .artifact-content a:hover {
          text-decoration: underline;
        }
        .artifact-content hr {
          border: none;
          border-top: 1px solid #e0e0e0;
          margin: 24px 0;
        }
      `}</style>
    </div>
  );
}
