import * as Dialog from "@radix-ui/react-dialog";
import { useState } from "react";
import { Outlet, useMatch } from "react-router";
import { CommandMenu } from "./CommandMenu";
import { CrumbProvider } from "./Crumbs";
import { Sidebar } from "./Sidebar";
import { Topbar } from "./Topbar";

export function AppShell() {
  const project = useMatch("/projects/:slug/*");
  const settings = useMatch("/projects/:slug/settings/*");
  const workspace = Boolean(project && !settings);
  const [search, setSearch] = useState(false);
  const [drawer, setDrawer] = useState(false);
  return (
    <CrumbProvider>
      <a className="skip-link" href="#main-content">Skip to content</a>
      <div className="app" data-workspace={workspace}>
        <Sidebar compact={workspace} />
        <div className="main">
          <Topbar onMenu={() => setDrawer(true)} onSearch={() => setSearch(true)} />
          <main id="main-content" className="main-content" tabIndex={-1}><Outlet /></main>
        </div>
      </div>
      <CommandMenu open={search} onOpenChange={setSearch} />
      <Dialog.Root open={drawer} onOpenChange={setDrawer}>
        <Dialog.Portal>
          <Dialog.Overlay className="overlay" />
          <Dialog.Content className="drawer" aria-describedby={undefined}>
            <Dialog.Title className="sr-only">Navigation</Dialog.Title>
            <Sidebar onNavigate={() => setDrawer(false)} />
          </Dialog.Content>
        </Dialog.Portal>
      </Dialog.Root>
    </CrumbProvider>
  );
}
