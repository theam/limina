import { CheckmarkFilled, Close, WarningFilled } from "@carbon/icons-react";
import { createContext, useCallback, useContext, useMemo, useState } from "react";

type ToastTone = "neutral" | "success" | "critical";
interface ToastItem {
  id: number;
  title: string;
  body?: string;
  tone: ToastTone;
}

const ToastContext = createContext<(title: string, options?: { body?: string; tone?: ToastTone }) => void>(
  () => undefined,
);

export function ToastProvider({ children }: { children: React.ReactNode }) {
  const [items, setItems] = useState<ToastItem[]>([]);
  const dismiss = useCallback((id: number) => setItems((all) => all.filter((item) => item.id !== id)), []);
  const push = useCallback(
    (title: string, options: { body?: string; tone?: ToastTone } = {}) => {
      const id = Date.now() + Math.random();
      setItems((all) => [...all.slice(-2), { id, title, body: options.body, tone: options.tone ?? "neutral" }]);
      window.setTimeout(() => dismiss(id), options.tone === "critical" ? 8000 : 4000);
    },
    [dismiss],
  );
  const value = useMemo(() => push, [push]);
  return (
    <ToastContext.Provider value={value}>
      {children}
      <div className="toast-region" role="status" aria-live="polite">
        {items.map((item) => (
          <div key={item.id} className="toast" data-tone={item.tone}>
            {item.tone === "critical" ? (
              <WarningFilled size={16} style={{ color: "rgb(var(--critical))", marginTop: 2 }} />
            ) : (
              <CheckmarkFilled size={16} style={{ color: "rgb(var(--success))", marginTop: 2 }} />
            )}
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{ fontWeight: 500 }}>{item.title}</div>
              {item.body && <div className="muted" style={{ marginTop: 2 }}>{item.body}</div>}
            </div>
            <button className="btn btn-ghost btn-icon btn-sm" onClick={() => dismiss(item.id)} aria-label="Dismiss">
              <Close size={16} />
            </button>
          </div>
        ))}
      </div>
    </ToastContext.Provider>
  );
}

export const useToast = () => useContext(ToastContext);
