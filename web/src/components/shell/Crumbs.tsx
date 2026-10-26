import { createContext, useContext, useEffect, useState } from "react";

export interface Crumb {
  label: string;
  to?: string;
}

const CrumbContext = createContext<{ crumbs: Crumb[]; set: (crumbs: Crumb[]) => void }>({
  crumbs: [],
  set: () => undefined,
});

export function CrumbProvider({ children }: { children: React.ReactNode }) {
  const [crumbs, set] = useState<Crumb[]>([]);
  return <CrumbContext.Provider value={{ crumbs, set }}>{children}</CrumbContext.Provider>;
}

/** Declare the breadcrumb trail for the current page (also sets the document title). */
export function useCrumbs(crumbs: Crumb[]): void {
  const { set } = useContext(CrumbContext);
  const signature = JSON.stringify(crumbs);
  useEffect(() => {
    set(crumbs);
    const last = crumbs[crumbs.length - 1]?.label;
    document.title = last ? `${last} · Limina` : "Limina";
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [signature, set]);
}

export const useCurrentCrumbs = () => useContext(CrumbContext).crumbs;
