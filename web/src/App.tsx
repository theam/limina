import { QueryClientProvider } from "@tanstack/react-query";
import * as Tooltip from "@radix-ui/react-tooltip";
import { useEffect } from "react";
import { Navigate, Outlet, RouterProvider, createBrowserRouter, useLocation, useNavigate } from "react-router";
import { AppShell } from "./components/shell/AppShell";
import { LiminaMark } from "./components/ui/Logo";
import { ToastProvider } from "./components/ui/Toast";
import { ApiError, onUnauthorized } from "./lib/api";
import { queryClient, useMe } from "./lib/queries";
import { Connections } from "./routes/Connections";
import { NewProject } from "./routes/NewProject";
import { NotFound } from "./routes/NotFound";
import { Projects } from "./routes/Projects";
import { SignIn } from "./routes/SignIn";
import { Workspace, WorkspaceState } from "./routes/project/Workspace";
import { WorkspaceDocument } from "./routes/project/WorkspaceDocument";
import { WorkspaceResults } from "./routes/project/WorkspaceResults";
import { KnowledgeHome } from "./routes/project/KnowledgeHome";
import { ProjectLayout } from "./routes/project/ProjectContext";
import {
  SettingsEngine,
  SettingsGeneral,
  SettingsMembers,
  SettingsResources,
  SettingsSources,
} from "./routes/project/Settings";
import { SettingsConnections } from "./routes/project/SettingsConnections";

function Splash({ children }: { children?: React.ReactNode }) {
  return (
    <div style={{ minHeight: "100dvh", display: "grid", placeItems: "center", padding: 24 }}>
      <div style={{ display: "grid", justifyItems: "center", gap: 16, textAlign: "center" }}>
        <LiminaMark size={36} />
        {children ?? <span className="spinner" />}
      </div>
    </div>
  );
}

function RequireSession() {
  const me = useMe();
  const navigate = useNavigate();
  const location = useLocation();

  useEffect(
    () => onUnauthorized(() => navigate("/signin", { state: { from: location.pathname }, replace: true })),
    [navigate, location.pathname],
  );

  if (me.isPending) return <Splash />;
  if (me.error instanceof ApiError && me.error.status === 401) {
    return <Navigate to="/signin" state={{ from: location.pathname }} replace />;
  }
  if (me.error) {
    return (
      <Splash>
        <div style={{ fontSize: "var(--text-lg)", fontWeight: 500 }}>Can’t reach the Limina runtime</div>
        <div className="muted" style={{ maxWidth: 380 }}>{me.error.message}</div>
        <button className="btn" onClick={() => me.refetch()}>
          Try again
        </button>
      </Splash>
    );
  }
  return <AppShell />;
}

function Home() {
  let last: string | null = null;
  try {
    last = localStorage.getItem("limina.lastProject");
  } catch {
    /* ignore */
  }
  return <Navigate to={last ? `/projects/${last}` : "/projects"} replace />;
}

const router = createBrowserRouter([
  { path: "/signin", element: <SignIn /> },
  {
    element: <RequireSession />,
    children: [
      { index: true, element: <Home /> },
      { path: "projects", element: <Projects /> },
      { path: "new", element: <NewProject /> },
      { path: "engines", element: <Navigate to="/projects" replace /> },
      { path: "connections", element: <Connections /> },
      {
        path: "projects/:slug",
        element: <ProjectLayout />,
        children: [
          { element: <Workspace />, children: [
            { index: true, element: <WorkspaceState /> },
            { path: "conversation", element: <Navigate to=".." replace /> },
            { path: "knowledge", element: <KnowledgeHome embedded /> },
            { path: "knowledge/:key", element: <WorkspaceDocument /> },
            { path: "results", element: <WorkspaceResults /> },
          ] },
          {
            path: "settings",
            element: <Outlet />,
            children: [
              { index: true, element: <SettingsGeneral /> },
              { path: "engine", element: <SettingsEngine /> },
              { path: "resources", element: <SettingsResources /> },
              { path: "sources", element: <SettingsSources /> },
              { path: "connections", element: <SettingsConnections /> },
              { path: "members", element: <SettingsMembers /> },
            ],
          },
        ],
      },
      { path: "*", element: <NotFound /> },
    ],
  },
]);

export function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <Tooltip.Provider delayDuration={400}>
        <ToastProvider>
          <RouterProvider router={router} />
        </ToastProvider>
      </Tooltip.Provider>
    </QueryClientProvider>
  );
}
