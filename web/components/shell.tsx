"use client";

import Link from "next/link";
import { usePathname, useRouter } from "next/navigation";
import {
  Activity,
  BarChart3,
  Boxes,
  KeyRound,
  LayoutDashboard,
  ListOrdered,
  LogOut,
  Settings,
  Users,
} from "lucide-react";

import { cn } from "@/lib/utils";
import { deleteCookie } from "@/lib/api";
import { Button } from "@/components/ui/button";

const NAV = [
  { href: "/", label: "仪表盘", icon: LayoutDashboard },
  { href: "/accounts", label: "账号管理", icon: Users },
  { href: "/keys", label: "密钥分发", icon: KeyRound },
  { href: "/models", label: "模型中心", icon: Boxes },
  { href: "/stats", label: "用量统计", icon: BarChart3 },
  { href: "/logs", label: "调用日志", icon: ListOrdered },
  { href: "/settings", label: "系统设置", icon: Settings },
];

export function Shell({ children }: { children: React.ReactNode }) {
  const pathname = usePathname();
  const router = useRouter();

  const logout = () => {
    deleteCookie("gw_key");
    router.push("/login");
  };

  const isActive = (href: string) =>
    href === "/" ? pathname === "/" : pathname.startsWith(href);

  return (
    <div className="flex min-h-screen">
      {/* 桌面端侧边栏（md 及以上显示） */}
      <aside className="hidden md:flex w-56 shrink-0 border-r border-[var(--border)] bg-[var(--card)] flex-col">
        <div className="px-5 py-5 border-b border-[var(--border)]">
          <div className="flex items-center gap-2">
            <Activity className="h-5 w-5 text-[var(--primary)]" />
            <div>
              <div className="text-sm font-semibold leading-tight">CodeArts 网关</div>
              <div className="text-xs text-[var(--muted-foreground)]">管理控制台</div>
            </div>
          </div>
        </div>
        <nav className="flex-1 p-3 space-y-1">
          {NAV.map((item) => (
            <Link
              key={item.href}
              href={item.href}
              className={cn(
                "flex items-center gap-3 rounded-md px-3 py-2 text-sm transition-colors",
                isActive(item.href)
                  ? "bg-[var(--primary)] text-[var(--primary-foreground)]"
                  : "text-[var(--foreground)] hover:bg-[var(--accent)]"
              )}
            >
              <item.icon className="h-4 w-4" />
              {item.label}
            </Link>
          ))}
        </nav>
        <div className="p-3 border-t border-[var(--border)]">
          <Button variant="ghost" size="sm" className="w-full justify-start" onClick={logout}>
            <LogOut className="h-4 w-4" />
            退出登录
          </Button>
        </div>
      </aside>

      {/* 主内容区 */}
      <div className="flex-1 min-w-0 flex flex-col">
        {/* 移动端顶部栏（md 以下显示） */}
        <div className="md:hidden sticky top-0 z-40 border-b border-[var(--border)] bg-[var(--card)]">
          <div className="flex items-center justify-between px-3 py-2.5">
            <div className="flex items-center gap-2">
              <Activity className="h-5 w-5 text-[var(--primary)]" />
              <span className="text-sm font-semibold">CodeArts 网关</span>
            </div>
            <Button variant="ghost" size="sm" onClick={logout}>
              <LogOut className="h-4 w-4" />
            </Button>
          </div>
          <nav className="flex gap-1 px-2 pb-2 overflow-x-auto">
            {NAV.map((item) => (
              <Link
                key={item.href}
                href={item.href}
                className={cn(
                  "flex items-center gap-1.5 rounded-md px-3 py-1.5 text-xs whitespace-nowrap transition-colors shrink-0",
                  isActive(item.href)
                    ? "bg-[var(--primary)] text-[var(--primary-foreground)]"
                    : "text-[var(--foreground)] hover:bg-[var(--accent)]"
                )}
              >
                <item.icon className="h-3.5 w-3.5" />
                {item.label}
              </Link>
            ))}
          </nav>
        </div>

        <main className="flex-1 min-w-0 p-3 md:p-6">{children}</main>
      </div>
    </div>
  );
}
