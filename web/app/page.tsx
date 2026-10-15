"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { motion } from "motion/react";
import { toast } from "sonner";
import {
  CheckCircle2,
  PauseCircle,
  RefreshCw,
  ShieldAlert,
  Users,
} from "lucide-react";
import {
  Cell,
  Legend,
  Pie,
  PieChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
  CartesianGrid,
  Area,
  AreaChart,
} from "recharts";

import { api, type AccountStatus, type RequestLog } from "@/lib/api";
import { Shell } from "@/components/shell";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";

const STATUS_COLORS: Record<string, string> = {
  "健康": "#22c55e",
  "冷却": "#f59e0b",
  "禁用": "#94a3b8",
};

export default function DashboardPage() {
  const [status, setStatus] = useState<AccountStatus[]>([]);
  const [logs, setLogs] = useState<RequestLog[]>([]);
  const [loading, setLoading] = useState(true);

  const load = useCallback(async () => {
    try {
      const [s, l] = await Promise.all([api.status(), api.logs()]);
      setStatus(s);
      setLogs(l);
    } catch (e) {
      if (e instanceof Error && e.message !== "unauthorized") {
        toast.error(`加载失败：${e.message}`);
      }
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const summary = useMemo(() => {
    const total = status.length;
    const healthy = status.filter((a) => a.healthy).length;
    const cooling = status.filter((a) => a.status === "cooling").length;
    const disabled = status.filter((a) => a.status === "disabled").length;
    return { total, healthy, cooling, disabled };
  }, [status]);

  const pieData = useMemo(
    () =>
      [
        { name: "健康", value: summary.healthy },
        { name: "冷却", value: summary.cooling },
        { name: "禁用", value: summary.disabled },
      ].filter((d) => d.value > 0),
    [summary]
  );

  const trendData = useMemo(() => {
    // Group recent logs by minute for the last ~30 entries.
    const recent = [...logs].slice(0, 60).reverse();
    return recent.map((l, i) => ({
      idx: i,
      time: new Date(l.ts).toLocaleTimeString("zh-CN", { hour12: false }),
      latency: l.latency_ms,
      status: l.status,
    }));
  }, [logs]);

  const cards = [
    { label: "账号总数", value: summary.total, icon: Users, color: "text-[var(--primary)]" },
    { label: "健康账号", value: summary.healthy, icon: CheckCircle2, color: "text-green-500" },
    { label: "冷却中", value: summary.cooling, icon: ShieldAlert, color: "text-amber-500" },
    { label: "已禁用", value: summary.disabled, icon: PauseCircle, color: "text-slate-400" },
  ];

  return (
    <Shell>
      <div className="flex items-center justify-between mb-6">
        <div>
          <h1 className="text-xl font-semibold">仪表盘</h1>
          <p className="text-sm text-[var(--muted-foreground)]">账号健康与请求概览</p>
        </div>
        <Button variant="outline" size="sm" onClick={load}>
          <RefreshCw className="h-4 w-4" />
          刷新
        </Button>
      </div>

      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4 mb-6">
        {cards.map((c, i) => (
          <motion.div
            key={c.label}
            initial={{ opacity: 0, y: 12 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ delay: i * 0.05, duration: 0.3 }}
          >
            <Card>
              <CardContent className="pt-6 flex items-center justify-between">
                <div>
                  <div className="text-sm text-[var(--muted-foreground)]">{c.label}</div>
                  {loading ? (
                    <Skeleton className="h-8 w-16 mt-1" />
                  ) : (
                    <div className="text-2xl font-bold mt-1">{c.value}</div>
                  )}
                </div>
                <c.icon className={`h-8 w-8 ${c.color}`} />
              </CardContent>
            </Card>
          </motion.div>
        ))}
      </div>

      <div className="grid gap-4 lg:grid-cols-2">
        <motion.div
          initial={{ opacity: 0, y: 12 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ delay: 0.2, duration: 0.3 }}
        >
          <Card>
            <CardHeader>
              <CardTitle>账号状态分布</CardTitle>
              <CardDescription>按健康状态统计</CardDescription>
            </CardHeader>
            <CardContent className="h-64">
              {loading ? (
                <Skeleton className="h-full w-full" />
              ) : pieData.length === 0 ? (
                <div className="h-full flex items-center justify-center text-sm text-[var(--muted-foreground)]">
                  暂无账号，请先前往账号管理添加
                </div>
              ) : (
                <ResponsiveContainer width="100%" height="100%">
                  <PieChart>
                    <Pie
                      data={pieData}
                      dataKey="value"
                      nameKey="name"
                      innerRadius={55}
                      outerRadius={85}
                      paddingAngle={3}
                    >
                      {pieData.map((d) => (
                        <Cell key={d.name} fill={STATUS_COLORS[d.name]} />
                      ))}
                    </Pie>
                    <Tooltip />
                    <Legend />
                  </PieChart>
                </ResponsiveContainer>
              )}
            </CardContent>
          </Card>
        </motion.div>

        <motion.div
          initial={{ opacity: 0, y: 12 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ delay: 0.25, duration: 0.3 }}
        >
          <Card>
            <CardHeader>
              <CardTitle>最近请求延迟</CardTitle>
              <CardDescription>按调用顺序展示（毫秒）</CardDescription>
            </CardHeader>
            <CardContent className="h-64">
              {loading ? (
                <Skeleton className="h-full w-full" />
              ) : trendData.length === 0 ? (
                <div className="h-full flex items-center justify-center text-sm text-[var(--muted-foreground)]">
                  暂无调用记录
                </div>
              ) : (
                <ResponsiveContainer width="100%" height="100%">
                  <AreaChart data={trendData}>
                    <CartesianGrid strokeDasharray="3 3" stroke="var(--border)" />
                    <XAxis dataKey="idx" tick={false} stroke="var(--muted-foreground)" />
                    <YAxis stroke="var(--muted-foreground)" fontSize={12} />
                    <Tooltip
                      formatter={((value: unknown) => [`${value}ms`, "延迟"]) as never}
                      labelFormatter={((idx: unknown) =>
                        trendData[Number(idx)]?.time ?? "") as never}
                    />
                    <Area
                      type="monotone"
                      dataKey="latency"
                      stroke="var(--primary)"
                      fill="var(--primary)"
                      fillOpacity={0.15}
                    />
                  </AreaChart>
                </ResponsiveContainer>
              )}
            </CardContent>
          </Card>
        </motion.div>
      </div>
    </Shell>
  );
}
