import {
  Chat,
  Checkmark,
  Grid,
  Logout,
  Plug,
  Settings,
  UserAvatar,
} from "@carbon/icons-react";
import * as Dropdown from "@radix-ui/react-dropdown-menu";
import { Link, NavLink, useLocation, useMatch, useNavigate } from "react-router";
import { session, signOutOfSession } from "../../lib/api";
import { useMe, queryClient } from "../../lib/queries";
import { useTheme, type ThemePreference } from "../../lib/theme";
import { LiminaMark } from "../ui/Logo";
import { ProjectSwitcher } from "./ProjectSwitcher";

function Item({
  to,
  icon,
  children,
  trailing,
  active,
  end,
  onNavigate,
}: {
  to: string;
  icon?: React.ReactNode;
  children: React.ReactNode;
  trailing?: React.ReactNode;
  active?: boolean;
  end?: boolean;
  onNavigate?: () => void;
}) {
  return (
    <NavLink
      to={to}
      end={end}
      className="nav-item"
      data-active={active || undefined}
      onClick={onNavigate}
    >
      {icon}
      <span>{children}</span>
      {trailing && <span className="nav-trailing">{trailing}</span>}
    </NavLink>
  );
}

function ProjectNav({ slug, onNavigate }: { slug: string; onNavigate?: () => void }) {
  const { pathname } = useLocation();
  const base = `/projects/${slug}`;
  const [section = ""] = pathname.slice(base.length).split("/").slice(1);
  return (
    <nav className="nav" aria-label="Project">
      <Item to={base} icon={<Chat size={16} />} active={section !== "settings"} onNavigate={onNavigate}>Agent workspace</Item>
      <Item
        to={`${base}/settings`}
        end
        icon={<Settings size={16} />}
        active={section === "settings"}
        onNavigate={onNavigate}
      >
        Settings
      </Item>
      {section === "settings" && (
        <div className="nav-sub">
          <Item to={`${base}/settings`} end onNavigate={onNavigate}>
            General
          </Item>
          <Item to={`${base}/settings/engine`} onNavigate={onNavigate}>
            Engine
          </Item>
          <Item to={`${base}/settings/resources`} onNavigate={onNavigate}>
            Resources
          </Item>
          <Item to={`${base}/settings/sources`} onNavigate={onNavigate}>
            Sources
          </Item>
          <Item to={`${base}/settings/connections`} onNavigate={onNavigate}>
            Connections
          </Item>
          <Item to={`${base}/settings/members`} onNavigate={onNavigate}>
            Members
          </Item>
        </div>
      )}
    </nav>
  );
}

const THEMES: { value: ThemePreference; label: string }[] = [
  { value: "system", label: "System" },
  { value: "light", label: "Light" },
  { value: "dark", label: "Dark" },
];

function AccountMenu() {
  const me = useMe();
  const [theme, setTheme] = useTheme();
  const navigate = useNavigate();
  const name = me.data?.display_name ?? "Account";
  return (
    <Dropdown.Root>
      <Dropdown.Trigger className="nav-item" style={{ width: "100%" }}>
        <UserAvatar size={16} />
        <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{name}</span>
      </Dropdown.Trigger>
      <Dropdown.Portal>
        <Dropdown.Content className="floating" side="top" align="start" sideOffset={6}>
          <div className="menu-label">
            <div style={{ fontWeight: 500 }}>{name}</div>
            <div className="muted" style={{ fontSize: "var(--text-sm)" }}>
              {me.data?.email ?? (me.data?.auth_mode === "local" ? "Local token session" : me.data?.subject)}
            </div>
          </div>
          <Dropdown.Separator className="menu-separator" />
          <Dropdown.Label className="menu-label label">Theme</Dropdown.Label>
          {THEMES.map((option) => (
            <Dropdown.Item key={option.value} className="menu-item" onSelect={() => setTheme(option.value)}>
              <span style={{ width: 16 }}>{theme === option.value && <Checkmark size={16} />}</span>
              {option.label}
            </Dropdown.Item>
          ))}
          <Dropdown.Separator className="menu-separator" />
          <Dropdown.Item
            className="menu-item"
            onSelect={async () => {
              if (me.data?.auth_mode === "workos") {
                // End the runtime session and the WorkOS session, like signing out of any app.
                window.location.assign(await signOutOfSession());
                return;
              }
              session.clear();
              queryClient.clear();
              navigate("/signin");
            }}
          >
            <Logout size={16} /> Sign out
          </Dropdown.Item>
        </Dropdown.Content>
      </Dropdown.Portal>
    </Dropdown.Root>
  );
}

export function Sidebar({ onNavigate, compact = false }: { onNavigate?: () => void; compact?: boolean }) {
  const slug = useMatch("/projects/:slug/*")?.params.slug;
  if (compact && slug) return <aside className="sidebar workspace-rail" aria-label="Navigation">
    <Link className="rail-brand" to="/projects" aria-label="Limina · All projects"><LiminaMark size={24} /></Link>
    <ProjectSwitcher slug={slug} />
    <nav aria-label="Project"><Link className="rail-item" to={`/projects/${slug}`} aria-label="Agent workspace" title="Agent workspace"><Chat size={20} /></Link><Link className="rail-item" to={`/projects/${slug}/settings`} aria-label="Project settings" title="Project settings"><Settings size={20} /></Link></nav>
    <nav className="rail-bottom" aria-label="Workspace"><Link className="rail-item" to="/projects" aria-label="All projects" title="All projects"><Grid size={20} /></Link><Link className="rail-item" to="/connections" aria-label="Connections" title="Connections"><Plug size={20} /></Link><AccountMenu /></nav>
  </aside>;
  return (
    <aside className="sidebar" aria-label="Navigation">
      <div className="sidebar-top">
        <Link to="/projects" className="brand" onClick={onNavigate}>
          <LiminaMark size={22} />
          Limina
        </Link>
        <ProjectSwitcher slug={slug} onNavigate={onNavigate} />
      </div>
      <div className="sidebar-scroll">
        {slug && <ProjectNav slug={slug} onNavigate={onNavigate} />}
        <nav className="nav" aria-label="Workspace">
          {slug && <div className="nav-heading label">Workspace</div>}
          <Item to="/projects" end icon={<Grid size={16} />} onNavigate={onNavigate}>
            All projects
          </Item>
          <Item to="/connections" icon={<Plug size={16} />} onNavigate={onNavigate}>
            Connections
          </Item>
        </nav>
      </div>
      <div className="sidebar-footer">
        <AccountMenu />
        <a className="signature" href="https://theagilemonkeys.com" target="_blank" rel="noreferrer">
          <LiminaMark size={14} />
          An initiative by The Agile Monkeys
        </a>
      </div>
    </aside>
  );
}
