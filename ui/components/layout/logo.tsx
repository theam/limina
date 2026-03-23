export function Logo({ size = 28, className }: { size?: number; className?: string }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 32 32"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      className={className}
    >
      {/* Magnifying glass body */}
      <circle
        cx="14"
        cy="14"
        r="9"
        stroke="currentColor"
        strokeWidth="2.5"
        strokeLinecap="round"
      />
      {/* Handle */}
      <line
        x1="21"
        y1="21"
        x2="28"
        y2="28"
        stroke="currentColor"
        strokeWidth="2.5"
        strokeLinecap="round"
      />
      {/* Sparkle / insight dot */}
      <circle cx="14" cy="11" r="1.5" fill="currentColor" opacity="0.5" />
      <circle cx="11" cy="15" r="1" fill="currentColor" opacity="0.35" />
    </svg>
  );
}
