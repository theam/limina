"use client";

import { useEffect, useState, useCallback, useRef } from "react";
import Nav from "../../components/nav";

interface LogEntry {
  timestamp: string;
  type: string;
  summary: string;
}

const MAX_ENTRIES = 1000;

const typeBadgeStyles: Record<
  string,
  { color: string; bg: string; label: string }
> = {
  assistant: { color: "#0f62fe", bg: "#edf5ff", label: "Agent" },
  tool: { color: "#198038", bg: "#defbe6", label: "Tool" },
  system: { color: "#525252", bg: "#f4f4f4", label: "System" },
  prompt: { color: "#6929c4", bg: "#f6f2ff", label: "Prompt" },
  directive: { color: "#da1e28", bg: "#fff1f1", label: "Directive" },
  result: { color: "#525252", bg: "#f4f4f4", label: "Done" },
};

function formatTime(iso: string): string {
  try {
    const d = new Date(iso);
    return d.toLocaleTimeString([], {
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    });
  } catch {
    return iso;
  }
}

export default function LogPage() {
  const [entries, setEntries] = useState<LogEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [stickToBottom, setStickToBottom] = useState(true);
  const scrollRef = useRef<HTMLDivElement>(null);
  const lastTimestampRef = useRef<string>("");

  const fetchLogs = useCallback(
    async (initial: boolean) => {
      try {
        const params = initial
          ? "tail=200"
          : `after=${encodeURIComponent(lastTimestampRef.current)}&tail=100`;
        const res = await fetch(`/api/log?${params}`);
        if (!res.ok) return;
        const data = await res.json();
        const newEntries: LogEntry[] = data.entries || [];

        if (newEntries.length === 0 && !initial) return;

        if (initial) {
          setEntries(newEntries.slice(-MAX_ENTRIES));
        } else {
          setEntries((prev) => {
            const merged = [...prev, ...newEntries];
            return merged.slice(-MAX_ENTRIES);
          });
        }

        if (newEntries.length > 0) {
          lastTimestampRef.current =
            newEntries[newEntries.length - 1].timestamp;
        }
      } catch {
        // Network error — skip this cycle
      } finally {
        if (initial) setLoading(false);
      }
    },
    []
  );

  useEffect(() => {
    fetchLogs(true);
    const interval = setInterval(() => fetchLogs(false), 3000);
    return () => clearInterval(interval);
  }, [fetchLogs]);

  // Auto-scroll when pinned to bottom
  useEffect(() => {
    if (stickToBottom && scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [entries, stickToBottom]);

  // Detect user scroll to disengage stick-to-bottom
  const handleScroll = useCallback(() => {
    if (!scrollRef.current) return;
    const { scrollTop, scrollHeight, clientHeight } = scrollRef.current;
    const atBottom = scrollHeight - scrollTop - clientHeight < 40;
    setStickToBottom(atBottom);
  }, []);

  return (
    <div
      style={{
        minHeight: "100vh",
        marginLeft: 64,
        backgroundColor: "#f4f4f4",
        fontFamily: "'IBM Plex Sans', sans-serif",
        display: "flex",
        flexDirection: "column",
      }}
    >
      <Nav activePath="/log" />

      {/* Header */}
      <div
        style={{
          maxWidth: 960,
          width: "100%",
          margin: "0 auto",
          padding: "24px 24px 0",
        }}
      >
        <div
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            marginBottom: 16,
          }}
        >
          <div>
            <h1
              style={{
                fontSize: 20,
                fontWeight: 600,
                color: "#161616",
                margin: 0,
              }}
            >
              Agent Log
            </h1>
            <p
              style={{
                fontSize: 13,
                color: "#525252",
                margin: "4px 0 0",
              }}
            >
              Live stream of agent activity — tool calls, reasoning, and
              session events.
            </p>
          </div>
          <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
            {entries.length > 0 && (
              <span
                style={{
                  fontSize: 12,
                  color: "#525252",
                  fontFamily: "'IBM Plex Mono', monospace",
                }}
              >
                {entries.length} entries
              </span>
            )}
            {!loading && entries.length > 0 && (
              <span
                style={{
                  width: 6,
                  height: 6,
                  borderRadius: "50%",
                  backgroundColor: "#198038",
                  animation: "logPulse 2s ease-in-out infinite",
                }}
              />
            )}
          </div>
        </div>
      </div>

      {/* Log stream */}
      <div
        style={{
          flex: 1,
          maxWidth: 960,
          width: "100%",
          margin: "0 auto",
          padding: "0 24px 24px",
          minHeight: 0,
        }}
      >
        {loading ? (
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
            Loading logs...
          </div>
        ) : entries.length === 0 ? (
          <div
            style={{
              backgroundColor: "#ffffff",
              border: "1px solid #e0e0e0",
              padding: 48,
              textAlign: "center",
            }}
          >
            <p
              style={{
                fontSize: 14,
                fontWeight: 500,
                color: "#161616",
                marginBottom: 8,
              }}
            >
              No agent activity yet
            </p>
            <p style={{ fontSize: 13, color: "#525252" }}>
              Logs will appear here when the research agent starts working.
            </p>
          </div>
        ) : (
          <div
            ref={scrollRef}
            onScroll={handleScroll}
            style={{
              backgroundColor: "#ffffff",
              border: "1px solid #e0e0e0",
              maxHeight: "calc(100vh - 160px)",
              overflowY: "auto",
              overflowX: "hidden",
            }}
          >
            {entries.map((entry, i) => {
              const badge = typeBadgeStyles[entry.type] ||
                typeBadgeStyles.system;
              return (
                <div
                  key={`${entry.timestamp}-${i}`}
                  style={{
                    display: "flex",
                    alignItems: "flex-start",
                    gap: 12,
                    padding: "8px 16px",
                    borderBottom: "1px solid #f4f4f4",
                    fontSize: 13,
                  }}
                >
                  <span
                    style={{
                      fontFamily: "'IBM Plex Mono', monospace",
                      fontSize: 11,
                      color: "#8d8d8d",
                      flexShrink: 0,
                      paddingTop: 2,
                    }}
                  >
                    {formatTime(entry.timestamp)}
                  </span>
                  <span
                    style={{
                      fontSize: 11,
                      fontWeight: 500,
                      color: badge.color,
                      backgroundColor: badge.bg,
                      padding: "1px 6px",
                      borderRadius: 2,
                      flexShrink: 0,
                      minWidth: 56,
                      textAlign: "center",
                    }}
                  >
                    {badge.label}
                  </span>
                  <span
                    style={{
                      color: "#161616",
                      lineHeight: 1.4,
                      wordBreak: "break-word",
                      flex: 1,
                    }}
                  >
                    {entry.summary}
                  </span>
                </div>
              );
            })}

            {/* Scroll anchor indicator */}
            {!stickToBottom && (
              <button
                onClick={() => {
                  setStickToBottom(true);
                  if (scrollRef.current) {
                    scrollRef.current.scrollTop =
                      scrollRef.current.scrollHeight;
                  }
                }}
                style={{
                  position: "sticky",
                  bottom: 8,
                  left: "50%",
                  transform: "translateX(-50%)",
                  display: "block",
                  margin: "0 auto",
                  padding: "4px 12px",
                  fontSize: 12,
                  fontWeight: 500,
                  fontFamily: "'IBM Plex Sans', sans-serif",
                  color: "#0f62fe",
                  backgroundColor: "#edf5ff",
                  border: "1px solid #a6c8ff",
                  borderRadius: 4,
                  cursor: "pointer",
                }}
              >
                Jump to latest
              </button>
            )}
          </div>
        )}
      </div>

      <style>{`
        @keyframes logPulse {
          0%, 100% { opacity: 1; }
          50% { opacity: 0.3; }
        }
      `}</style>
    </div>
  );
}
