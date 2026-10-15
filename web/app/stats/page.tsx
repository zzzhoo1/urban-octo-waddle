"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { motion } from "motion/react";
import { toast } from "sonner";
import {
  Activity,
  CheckCircle2,
  Coins,
  RefreshCw,
  Timer,
  Zap,
} from "lucide-react";
import {
  Area,
  AreaChart,
  Bar,
  BarChart,
  CartesianGrid,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";

import { api, type StatsResponse } from "@/lib/api";
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

function fmtNum(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return String(n);
}

export default function StatsPage() {
  const [stats, setStats] = useState<StatsResponse | null>(null);
  const [loading, setLoading] = useState(true);

  const load = useCallback(async () => {
    try {
      setStats(await api.stats());
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

  const cards = useMemo(() => {
    if (!stats) return [];
    const t = stats.totals;
    return [
      { label: "请求总量", value: fmtNum(t.requests), icon: Activity, color: "text-[var(--primary)]" },
      { label: "成功率", value: `${(t.success_rate * 100).toFixed(1)}%`, icon: CheckCircle2, color: "text-green-500" },
      { label: "Token 消耗", value: fmtNum(t.total_tokens), icon: Coins, color: "text-amber-500" },
      { label: "平均延迟", value: `${t.avg_latency_ms}ms`, icon: Timer, color: "text-sky-500" },
      { label: "平均首字延迟", value: t.avg_ttft_ms ? `${t.avg_ttft_ms}ms` : "-", icon: Zap, color: "text-violet-500" },
    ];
  }, [stats]);

  const empty = !loading && stats && stats.totals.requests === 0;

  return (
    <Shell>
      <div className="flex items-center justify-between mb-6">
        <div>
          <h1 className="text-xl font-semibold">用量统计</h1>
          <p className="text-sm text-[var(--muted-foreground)]">
            基于内存中的调用记录聚合，重启后重新累计
          </p>
        </div>
        <Button variant="outline" size="sm" onClick={load}>
          <RefreshCw className="h-4 w-4" />
          刷新
        </Button>
      </div>

      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-5 mb-6">
        {(loading ? Array.from({ length: 5 }) : cards).map((c, i) => (
          <motion.div
            key={i}
            initial={{ opacity: 0, y: 12 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ delay: i * 0.05, duration: 0.3 }}
          >
            <Card>
              <CardContent className="pt-6">
                {loading || !('label' in c) ? (
                  <Skeleton className="h-14 w-full" />
                ) : (
                  <div className="flex items-center justify-between">
                    <div>
                      <div className="text-xs text-[var(--muted-foreground)]">{c.label}</div>
                      <div className="text-xl font-bold mt-1">{c.value}</div>
                    </div>
                    <c.icon className={`h-7 w-7 ${c.color}`} />
                  </div>
                )}
              </CardContent>
            </Card>
          </motion.div>
        ))}
      </div>

      {empty ? (
        <Card>
          <CardContent className="py-16 text-center text-sm text-[var(--muted-foreground)]">
            暂无调用数据，有请求经过网关后这里会出现统计
          </CardContent>
        </Card>
      ) : (
        <div className="grid gap-4 lg:grid-cols-2">
          <motion.div initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0 }} transition={{ delay: 0.2, duration: 0.3 }}>
            <Card>
              <CardHeader>
                <CardTitle>按天用量趋势</CardTitle>
                <CardDescription>请求数与 token 消耗</CardDescription>
              </CardHeader>
              <CardContent className="h-64">
                {loading ? (
                  <Skeleton className="h-full w-full" />
                ) : (
                  <ResponsiveContainer width="100%" height="100%">
                    <AreaChart data={stats?.by_day ?? []}>
                      <CartesianGrid strokeDasharray="3 3" stroke="var(--border)" />
                      <XAxis dataKey="date" stroke="var(--muted-foreground)" fontSize={11} />
                      <YAxis stroke="var(--muted-foreground)" fontSize={12} />
                      <Tooltip />
                      <Area type="monotone" dataKey="requests" name="请求数" stroke="var(--primary)" fill="var(--primary)" fillOpacity={0.15} />
                      <Area type="monotone" dataKey="total_tokens" name="Token" stroke="#f59e0b" fill="#f59e0b" fillOpacity={0.1} />
                    </AreaChart>
                  </ResponsiveContainer>
                )}
              </CardContent>
            </Card>
          </motion.div>

          <motion.div initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0 }} transition={{ delay: 0.25, duration: 0.3 }}>
            <Card>
              <CardHeader>
                <CardTitle>按模型分布</CardTitle>
                <CardDescription>各模型的请求量</CardDescription>
              </CardHeader>
              <CardContent className="h-64">
                {loading ? (
                  <Skeleton className="h-full w-full" />
                ) : (
                  <ResponsiveContainer width="100%" height="100%">
                    <BarChart data={stats?.by_model ?? []}>
                      <CartesianGrid strokeDasharray="3 3" stroke="var(--border)" />
                      <XAxis dataKey="model" stroke="var(--muted-foreground)" fontSize={11} />
                      <YAxis stroke="var(--muted-foreground)" fontSize={12} />
                      <Tooltip />
                      <Bar dataKey="requests" name="请求数" fill="var(--primary)" radius={[4, 4, 0, 0]} />
                    </BarChart>
                  </ResponsiveContainer>
                )}
              </CardContent>
            </Card>
          </motion.div>

          <motion.div initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0 }} transition={{ delay: 0.3, duration: 0.3 }} className="lg:col-span-2">
            <Card>
              <CardHeader>
                <CardTitle>按密钥分布</CardTitle>
                <CardDescription>各密钥的请求量与 token 消耗</CardDescription>
              </CardHeader>
              <CardContent className="h-56">
                {loading ? (
                  <Skeleton className="h-full w-full" />
                ) : (
                  <ResponsiveContainer width="100%" height="100%">
                    <BarChart data={stats?.by_key ?? []} layout="vertical">
                      <CartesianGrid strokeDasharray="3 3" stroke="var(--border)" />
                      <XAxis type="number" stroke="var(--muted-foreground)" fontSize={12} />
                      <YAxis type="category" dataKey="key_name" stroke="var(--muted-foreground)" fontSize={12} width={120} />
                      <Tooltip />
                      <Bar dataKey="requests" name="请求数" fill="var(--primary)" radius={[0, 4, 4, 0]} />
                      <Bar dataKey="total_tokens" name="Token" fill="#f59e0b" radius={[0, 4, 4, 0]} />
                    </BarChart>
                  </ResponsiveContainer>
                )}
              </CardContent>
            </Card>
          </motion.div>
        </div>
      )}
    </Shell>
  );
}
