"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { cn } from "@/lib/utils";
import {
  LayoutDashboard,
  CheckSquare,
  FlaskConical,
  Wrench,
  Shield,
  Scale,
  PanelLeftClose,
  PanelLeft,
} from "lucide-react";
import { useState } from "react";
import { Logo } from "./logo";

const NAV_ITEMS = [
  { href: "/", label: "Dashboard", icon: LayoutDashboard },
  { href: "/tasks", label: "Tasks", icon: CheckSquare },
  { href: "/research", label: "Research", icon: FlaskConical },
  { href: "/engineering", label: "Engineering", icon: Wrench },
  { href: "/reviews", label: "Reviews", icon: Shield },
  { href: "/decisions", label: "Decisions", icon: Scale },
];

export function Sidebar() {
  const pathname = usePathname();
  const [collapsed, setCollapsed] = useState(false);

  return (
    <aside
      className={cn(
        "sticky top-0 h-screen flex flex-col border-r border-stone-200 bg-stone-50/80 transition-all duration-200",
        collapsed ? "w-16" : "w-56",
      )}
    >
      {/* Header */}
      <div className="flex items-center gap-2 px-4 py-5 border-b border-stone-200">
        <Logo size={24} className="shrink-0 text-stone-700" />
        {!collapsed && (
          <div className="flex-1 min-w-0">
            <h1 className="text-sm font-semibold text-stone-900 truncate">
              Researcher
            </h1>
          </div>
        )}
        <button
          onClick={() => setCollapsed(!collapsed)}
          className="p-1 rounded-md text-stone-400 hover:text-stone-600 hover:bg-stone-100 transition-colors"
        >
          {collapsed ? <PanelLeft size={18} /> : <PanelLeftClose size={18} />}
        </button>
      </div>

      {/* Navigation */}
      <nav className="flex-1 px-2 py-3 space-y-0.5">
        {NAV_ITEMS.map(({ href, label, icon: Icon }) => {
          const isActive =
            href === "/"
              ? pathname === "/"
              : pathname.startsWith(href);
          return (
            <Link
              key={href}
              href={href}
              className={cn(
                "flex items-center gap-3 px-3 py-2 rounded-lg text-sm transition-colors",
                isActive
                  ? "bg-stone-200/80 text-stone-900 font-medium"
                  : "text-stone-500 hover:text-stone-700 hover:bg-stone-100",
                collapsed && "justify-center px-0",
              )}
              title={collapsed ? label : undefined}
            >
              <Icon size={18} className="shrink-0" />
              {!collapsed && <span>{label}</span>}
            </Link>
          );
        })}
      </nav>

      {/* Footer */}
      <div className="px-4 py-3 border-t border-stone-200">
        {!collapsed && (
          <p className="text-xs text-stone-400">Autonomous Researcher</p>
        )}
      </div>
    </aside>
  );
}
