"use client";

import Link from "next/link";
import { useEffect, useState } from "react";

type MissionStatus = "Running" | "Completed" | "Failed" | "Blocked" | "Idle";

interface NavProps {
  activePath: string;
}

// DESIGN.md: "prefer shallow, understandable structures"
// Icons are minimal SVG — no icon library dependency
const tabs = [
  {
    label: "Dashboard",
    href: "/",
    icon: (
      <svg width="20" height="20" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5">
        <rect x="2" y="2" width="7" height="7" rx="1" />
        <rect x="11" y="2" width="7" height="4" rx="1" />
        <rect x="2" y="11" width="7" height="4" rx="1" />
        <rect x="11" y="8" width="7" height="7" rx="1" />
      </svg>
    ),
  },
  {
    label: "Research",
    href: "/research",
    icon: (
      <svg width="20" height="20" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5">
        <circle cx="8" cy="8" r="5" />
        <path d="M12 12L17 17" />
        <path d="M6 8h4M8 6v4" />
      </svg>
    ),
  },
  {
    label: "Findings",
    href: "/findings",
    icon: (
      <svg width="20" height="20" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5">
        <path d="M3 15l4-6 3 4 3-8 4 10" />
        <path d="M3 17h14" />
      </svg>
    ),
  },
  {
    label: "Steering",
    href: "/steering",
    icon: (
      <svg width="20" height="20" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5">
        <circle cx="10" cy="10" r="7" />
        <path d="M10 3v2M10 15v2M3 10h2M15 10h2" />
        <circle cx="10" cy="10" r="2" />
      </svg>
    ),
  },
  {
    label: "Cost",
    href: "/cost",
    icon: (
      <svg width="20" height="20" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5">
        <path d="M10 2v16M6 5h6a2 2 0 010 4H7a2 2 0 000 4h7" />
      </svg>
    ),
  },
  {
    label: "Report",
    href: "/report",
    icon: (
      <svg width="20" height="20" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5">
        <rect x="3" y="2" width="14" height="16" rx="1" />
        <path d="M6 6h8M6 9h8M6 12h5" />
      </svg>
    ),
  },
];

const statusColors: Record<MissionStatus, string> = {
  Running: "#198038",
  Completed: "#0f62fe",
  Failed: "#da1e28",
  Blocked: "#8e6a00",
  Idle: "#525252",
};

const statusBgColors: Record<MissionStatus, string> = {
  Running: "#defbe6",
  Completed: "#edf5ff",
  Failed: "#fff1f1",
  Blocked: "#fef3cd",
  Idle: "#f4f4f4",
};

function deriveStatus(phase: string | undefined): MissionStatus {
  if (!phase) return "Idle";
  if (phase === "COMPLETED") return "Completed";
  if (phase === "BLOCKED") return "Blocked";
  if (phase === "FAILED_RECOVERABLE" || phase === "KILLED") return "Failed";
  if (["RUNNING", "PLANNING", "CHECKPOINT_WAITING", "ESCALATION_WAITING", "STALLED"].includes(phase))
    return "Running";
  return "Idle";
}

export default function Nav({ activePath }: NavProps) {
  const [status, setStatus] = useState<MissionStatus>("Idle");
  const [budget, setBudget] = useState<string | null>(null);

  useEffect(() => {
    const fetchStatus = async () => {
      try {
        const res = await fetch("/api/status");
        if (!res.ok) return;
        const data = await res.json();
        if (data.mission) {
          setStatus(deriveStatus(data.mission.phase));
          setBudget(data.mission.budget || data.mission.estimatedCost || null);
        }
      } catch {
        // ignore
      }
    };
    fetchStatus();
    const interval = setInterval(fetchStatus, 10000);
    return () => clearInterval(interval);
  }, []);

  return (
    <nav
      style={{
        position: "fixed",
        top: 0,
        left: 0,
        bottom: 0,
        width: 64,
        backgroundColor: "#ffffff",
        borderRight: "1px solid var(--border-subtle, #e0e0e0)",
        fontFamily: "'IBM Plex Sans', sans-serif",
        display: "flex",
        flexDirection: "column",
        zIndex: 50,
      }}
    >
      {/* Brand mark — Limina icon */}
      <div
        style={{
          height: 64,
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          borderBottom: "1px solid var(--border-subtle, #e0e0e0)",
        }}
      >
        <img
          src="/icon-192.png"
          alt="Limina"
          style={{ width: 28, height: 28, objectFit: "contain" }}
        />
      </div>

      {/* Nav items */}
      <div
        style={{
          flex: 1,
          display: "flex",
          flexDirection: "column",
          paddingTop: 8,
          gap: 2,
        }}
      >
        {tabs.map((tab) => {
          const isActive = activePath === tab.href;
          return (
            <Link
              key={tab.href}
              href={tab.href}
              title={tab.label}
              style={{
                display: "flex",
                flexDirection: "column",
                alignItems: "center",
                justifyContent: "center",
                padding: "10px 0",
                margin: "0 8px",
                borderRadius: 6,
                color: isActive
                  ? "var(--interactive, #0f62fe)"
                  : "var(--text-secondary, #525252)",
                backgroundColor: isActive ? "#edf5ff" : "transparent",
                textDecoration: "none",
                transition: "background-color 150ms, color 150ms",
                gap: 2,
              }}
            >
              {tab.icon}
              <span
                style={{
                  fontSize: 9,
                  fontWeight: isActive ? 600 : 400,
                  letterSpacing: "0.02em",
                }}
              >
                {tab.label}
              </span>
            </Link>
          );
        })}
      </div>

      {/* Status badge at bottom */}
      <div
        style={{
          padding: "12px 8px",
          borderTop: "1px solid var(--border-subtle, #e0e0e0)",
          display: "flex",
          flexDirection: "column",
          alignItems: "center",
          gap: 6,
        }}
      >
        <div
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            gap: 4,
          }}
        >
          <span
            style={{
              width: 6,
              height: 6,
              borderRadius: "50%",
              backgroundColor: statusColors[status],
              ...(status === "Running"
                ? { animation: "navPulse 2s ease-in-out infinite" }
                : {}),
            }}
          />
          <span
            style={{
              fontSize: 9,
              fontWeight: 500,
              color: statusColors[status],
            }}
          >
            {status}
          </span>
        </div>
        {budget && (
          <span
            style={{
              fontSize: 9,
              color: "var(--text-secondary, #525252)",
              textAlign: "center",
            }}
          >
            {budget}
          </span>
        )}
      </div>

      <style>{`
        @keyframes navPulse {
          0%, 100% { opacity: 1; }
          50% { opacity: 0.4; }
        }
        nav a:hover {
          background-color: #f4f4f4 !important;
        }
      `}</style>
    </nav>
  );
}
