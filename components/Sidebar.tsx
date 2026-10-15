"use client";

import React from "react";
import Link from "next/link";
import { usePathname } from "next/navigation";
import { Menu } from "lucide-react";
import * as Dialog from "@radix-ui/react-dialog";
import * as Tooltip from "@radix-ui/react-tooltip";
import create from "zustand";
import { persist } from "zustand/middleware";
import clsx from "clsx";
import { Icons, IconName } from "@/components/icons";

// --- Sidebar state (persisted) ---
type SidebarState = {
  collapsed: boolean;
  toggle: () => void;
  setCollapsed: (v: boolean) => void;
};

const useSidebarStore = create<SidebarState>()(
  persist(
    (set) => ({
      collapsed: false,
      toggle: () => set((s) => ({ collapsed: !s.collapsed })),
      setCollapsed: (v: boolean) => set({ collapsed: v }),
    }),
    { name: "sidebar-ui-v1" }
  )
);

// --- Navigation definition ---
const NAV: { href: string; label: string; icon: IconName }[] = [
  { href: "/", label: "仪表盘", icon: "PieChart" },
  { href: "/accounts", label: "账号管理", icon: "Users" },
  { href: "/keys", label: "密钥分发", icon: "Key" },
  { href: "/models", label: "模型中心", icon: "Cpu" },
  { href: "/stats", label: "用量统计", icon: "BarChart" },
  { href: "/logs", label: "调用日志", icon: "List" },
  { href: "/tasks", label: "任务记录", icon: "Clock" },
  { href: "/settings", label: "系统设置", icon: "Settings" },
];

function Icon({ name, className = "w-5 h-5" }: { name: IconName; className?: string }) {
  const Comp = Icons[name] ?? Icons.PieChart;
  return <Comp className={clsx(className)} aria-hidden />;
}

export default function Sidebar() {
  const pathname = usePathname() ?? "/";
  const { collapsed, toggle, setCollapsed } = useSidebarStore();

  // Mobile dialog state (Radix)
  const [open, setOpen] = React.useState(false);

  React.useEffect(() => {
    // auto-collapse on narrow widths
    const handler = () => {
      if (window.innerWidth < 768) setCollapsed(true);
    };
    handler();
    window.addEventListener("resize", handler);
    return () => window.removeEventListener("resize", handler);
  }, [setCollapsed]);

  return (
    <>
      {/* Mobile: hamburger opens sheet dialog */}
      <div className="md:hidden">
        <div className="flex items-center gap-2 px-3 py-2">
          <Dialog.Root open={open} onOpenChange={setOpen}>
            <Dialog.Trigger asChild>
              <button
                aria-label="打开导航"
                className="p-2 rounded-md hover:bg-accent focus:outline-none focus-visible:ring"
              >
                <Menu className="w-5 h-5" />
              </button>
            </Dialog.Trigger>

            <Dialog.Portal>
              <Dialog.Overlay className="fixed inset-0 bg-black/40" />
              <Dialog.Content className="fixed left-0 top-0 h-full w-72 bg-background shadow-lg p-4">
                <nav aria-label="移动导航" className="flex flex-col gap-2">
                  <div className="mb-4 flex items-center justify-between">
                    <div className="font-semibold">AI 网关</div>
                    <Dialog.Close asChild>
                      <button className="p-1 rounded hover:bg-accent">
                        <Icons.LogOut className="w-4 h-4" />
                      </button>
                    </Dialog.Close>
                  </div>

                  {NAV.map((item) => {
                    const active = pathname === item.href || pathname.startsWith(item.href + "/");
                    return (
                      <Link key={item.href} href={item.href} onClick={() => setOpen(false)}>
                        <a
                          className={clsx(
                            "flex items-center gap-3 rounded-md px-3 py-2 text-sm",
                            active ? "bg-primary/10 text-primary" : "hover:bg-accent"
                          )}
                          aria-current={active ? "page" : undefined}
                        >
                          <Icon name={item.icon} />
                          <span>{item.label}</span>
                        </a>
                      </Link>
                    );
                  })}
                </nav>
              </Dialog.Content>
            </Dialog.Portal>
          </Dialog.Root>

          <div className="ml-auto flex items-center gap-2">
            <button
              className="p-2 rounded-md hover:bg-accent focus-visible:ring"
              aria-pressed={collapsed}
              onClick={() => toggle()}
            >
              <Menu className="w-5 h-5" />
            </button>
          </div>
        </div>
      </div>

      {/* Desktop sidebar */}
      <aside
        className={clsx(
          "hidden md:flex flex-col h-screen sticky top-0 border-r border-border bg-surface",
          collapsed ? "w-16" : "w-56",
          "transition-all duration-200"
        )}
        aria-label="主��航"
      >
        <div className="flex flex-col h-full">
          {/* header */}
          <div className="flex items-center gap-3 px-3 py-4">
            <button
              onClick={() => toggle()}
              aria-pressed={collapsed}
              className="p-2 rounded-md hover:bg-accent focus-visible:ring"
            >
              <Menu className="w-5 h-5" />
            </button>
            {!collapsed && <div className="font-semibold">AI 网关</div>}
          </div>

          {/* nav */}
          <nav className="flex-1 overflow-y-auto px-1 py-2" aria-label="侧边导航">
            <ul className="space-y-1">
              {NAV.map((item) => {
                const active = pathname === item.href || pathname.startsWith(item.href + "/");
                return (
                  <li key={item.href}>
                    <Tooltip.Provider delayDuration={100}>
                      <Tooltip.Root>
                        <Tooltip.Trigger asChild>
                          <Link href={item.href} className="group block">
                            <a
                              className={clsx(
                                "flex items-center gap-3 rounded-md px-2 py-2 text-sm",
                                active ? "bg-primary/10 text-primary" : "text-foreground hover:bg-accent"
                              )}
                              aria-current={active ? "page" : undefined}
                            >
                              <span className="flex-shrink-0">
                                <Icon name={item.icon} />
                              </span>

                              {!collapsed ? (
                                <span className="truncate">{item.label}</span>
                              ) : (
                                <span className="sr-only">{item.label}</span>
                              )}
                            </a>
                          </Link>
                        </Tooltip.Trigger>

                        {collapsed && (
                          <Tooltip.Portal>
                            <Tooltip.Content side="right" className="rounded-md bg-black text-white px-2 py-1 text-xs">
                              {item.label}
                              <Tooltip.Arrow className="fill-current text-black" />
                            </Tooltip.Content>
                          </Tooltip.Portal>
                        )}
                      </Tooltip.Root>
                    </Tooltip.Provider>
                  </li>
                );
              })}
            </ul>
          </nav>

          {/* footer */}
          <div className="px-3 py-3">
            <button className="flex items-center gap-2 w-full rounded-md px-2 py-2 text-sm hover:bg-accent">
              <Icons.LogOut className="w-4 h-4" />
              {!collapsed && <span>退出登录</span>}
            </button>
          </div>
        </div>
      </aside>
    </>
  );
}
