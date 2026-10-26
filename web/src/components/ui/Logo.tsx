/** Limina mark: a threshold (door frame) with light passing through it in yellow. */
export function LiminaMark({ size = 24 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 32 32" aria-hidden="true">
      <rect width="32" height="32" rx="6" fill="rgb(var(--fg))" />
      <path
        d="M10 25V9.5A1.5 1.5 0 0 1 11.5 8h9A1.5 1.5 0 0 1 22 9.5V25"
        fill="none"
        stroke="rgb(var(--bg))"
        strokeWidth="2.5"
      />
      <path d="M16 25V14" stroke="rgb(var(--focus))" strokeWidth="2.5" />
    </svg>
  );
}
