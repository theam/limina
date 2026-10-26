import { useCallback, useSyncExternalStore } from "react";

export type ThemePreference = "system" | "light" | "dark";
const KEY = "limina.theme";
const listeners = new Set<() => void>();

function current(): ThemePreference {
  const value = document.documentElement.dataset.theme;
  return value === "light" || value === "dark" ? value : "system";
}

export function setTheme(preference: ThemePreference): void {
  if (preference === "system") delete document.documentElement.dataset.theme;
  else document.documentElement.dataset.theme = preference;
  try {
    if (preference === "system") localStorage.removeItem(KEY);
    else localStorage.setItem(KEY, preference);
  } catch {
    /* preference lasts for this page only */
  }
  listeners.forEach((listener) => listener());
}

export function useTheme(): [ThemePreference, (value: ThemePreference) => void] {
  const theme = useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    current,
    () => "system" as ThemePreference,
  );
  return [theme, useCallback(setTheme, [])];
}
