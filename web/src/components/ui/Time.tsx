import { useEffect, useState } from "react";
import { fullDate, timeAgo } from "../../lib/format";

/** Relative timestamp that stays fresh and exposes the exact time on hover. */
export function Time({ iso, className }: { iso: string | null | undefined; className?: string }) {
  const [, setTick] = useState(0);
  useEffect(() => {
    const timer = window.setInterval(() => setTick((value) => value + 1), 30_000);
    return () => window.clearInterval(timer);
  }, []);
  if (!iso) return <span className={className}>—</span>;
  return (
    <time dateTime={iso} title={fullDate(iso)} className={className}>
      {timeAgo(iso)}
    </time>
  );
}

/** Live elapsed counter, e.g. for an active turn. */
export function Elapsed({ since }: { since: string }) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, []);
  const seconds = Math.max(0, Math.round((now - new Date(since).getTime()) / 1000));
  const minutes = Math.floor(seconds / 60);
  return (
    <span className="tabular">
      {minutes ? `${minutes}m ${String(seconds % 60).padStart(2, "0")}s` : `${seconds}s`}
    </span>
  );
}
