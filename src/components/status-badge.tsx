"use client";

// DESIGN.md: "no color-only meaning — status badges use color + text label"
// Colors from the plan's DESIGN.md alignment section

interface StatusBadgeProps {
  status: string;
  size?: "sm" | "md";
}

const statusStyles: Record<string, { bg: string; color: string }> = {
  // Hypothesis statuses
  PROPOSED: { bg: "#f4f4f4", color: "#525252" },
  TESTING: { bg: "#edf5ff", color: "#0f62fe" },
  CONFIRMED: { bg: "#defbe6", color: "#198038" },
  REJECTED: { bg: "#fff1f1", color: "#da1e28" },

  // Experiment statuses
  DESIGNED: { bg: "#f4f4f4", color: "#525252" },
  RUNNING: { bg: "#edf5ff", color: "#0f62fe" },
  COMPLETED: { bg: "#defbe6", color: "#198038" },
  FAILED: { bg: "#fff1f1", color: "#da1e28" },

  // Task statuses
  BACKLOG: { bg: "#f4f4f4", color: "#525252" },
  TODO: { bg: "#f4f4f4", color: "#525252" },
  IN_PROGRESS: { bg: "#edf5ff", color: "#0f62fe" },
  DONE: { bg: "#defbe6", color: "#198038" },
  BLOCKED: { bg: "#fff1f1", color: "#da1e28" },

  // Impact levels
  HIGH: { bg: "#defbe6", color: "#198038" },
  MEDIUM: { bg: "#fef3cd", color: "#8e6a00" },
  LOW: { bg: "#f4f4f4", color: "#525252" },

  // Fallback
  unknown: { bg: "#f4f4f4", color: "#525252" },
};

export default function StatusBadge({ status, size = "sm" }: StatusBadgeProps) {
  const normalized = status?.toUpperCase() || "unknown";
  const style = statusStyles[normalized] || statusStyles.unknown;

  const fontSize = size === "sm" ? 11 : 12;
  const padding = size === "sm" ? "2px 8px" : "3px 10px";

  return (
    <span
      style={{
        display: "inline-flex",
        alignItems: "center",
        fontSize,
        fontWeight: 500,
        fontFamily: "'IBM Plex Sans', sans-serif",
        color: style.color,
        backgroundColor: style.bg,
        padding,
        borderRadius: 4,
        whiteSpace: "nowrap",
      }}
    >
      {status}
    </span>
  );
}
