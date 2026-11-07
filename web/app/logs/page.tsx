"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { motion } from "motion/react";
import { toast } from "sonner";
import { RefreshCw } from "lucide-react";

import { api, type RequestLog } from "@/lib/api";
import { formatLatency, formatTime } from "@/lib/utils";
import { Shell } from "@/components/shell";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";

function statusBadge(status: number) {
  if (status < 300) return <Badge variant="success">{status}</Badge>;
  if (status === 401 || status === 403) return <Badge variant="warning">{status}</Badge>;
  if (status === 429) return <Badge variant="warning">{status}</Badge>;
  return <Badge variant="destructive">{status}</Badge>;
}

/// workbuddy-manager semantics: unknown values render as "—", never 0.
function dash(v?: number | null, fmt?: (n: number) => string): string {
  if (v === undefined || v === null) return "—";
  return fmt ? fmt(v) : String(v);
}

const RANGES = [
  { key: "1h", label: "最近 1 小时", ms: 3600_000 },
  { key: "today", label: "今天", ms: 86400_000 },
  { key: "7d", label: "最近 7 天", ms: 7 * 86400_000 },
  { key: "30d", label: "最近 30 天", ms: 30 * 86400_000 },
  { key: "all", label: "全部", ms: Infinity },
] as const;

const PAGE_LIMIT = 200;

/** 移动端纵向卡片：一条日志一张卡，字段逐行展示 */
function LogCard({ log }: { log: RequestLog }) {
  const rows: Array<[string, React.ReactNode]> = [
    ["模型", <span key="m" className="font-medium">{log.model || "-"}</span>],
    ["状态码", statusBadge(log.status)],
    ["密钥", log.key_name || "-"],
    ["账号", log.account_id || "-"],
    ["解析层级", log.route_via || "-"],
    ["尝试次数", log.attempts && log.attempts > 1 ? `${log.attempts} 次` : "1 次"],
    ["来源 IP", <span key="ip" className="font-mono text-xs">{log.source_ip || "-"}</span>],
    ["首字延迟", dash(log.ttft_ms && log.ttft_ms > 0 ? log.ttft_ms : undefined, formatLatency)],
    ["总耗时", formatLatency(log.latency_ms)],
    ["Token", log.total_tokens ? log.total_tokens.toLocaleString() : "—"],
    ["实际扣费", dash(log.credit)],
    ["缓存命中", dash(log.cached_tokens, (n) => n.toLocaleString())],
  ];
  return (
    <div className="rounded-lg border border-[var(--border)] bg-[var(--card)] p-3 space-y-1.5">
      <div className="text-xs text-[var(--muted-foreground)] pb-1 border-b border-[var(--border)]">
        {formatTime(log.ts)}
      </div>
      {rows.map(([label, value]) => (
        <div key={label} className="flex items-center justify-between text-sm">
          <span className="text-[var(--muted-foreground)] text-xs">{label}</span>
          <span>{value}</span>
        </div>
      ))}
    </div>
  );
}

export default function LogsPage() {
  const [logs, setLogs] = useState<RequestLog[]>([]);
  const [loading, setLoading] = useState(true);
  const [range, setRange] = useState<(typeof RANGES)[number]["key"]>("today");
  const timer = useRef<ReturnType<typeof setInterval> | null>(null);

  const load = useCallback(async (silent = false) => {
    try {
      const data = await api.logs();
      setLogs(data);
    } catch (e) {
      if (!silent && e instanceof Error && e.message !== "unauthorized") {
        toast.error(`加载失败：${e.message}`);
      }
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    load();
    timer.current = setInterval(() => load(true), 30_000);
    return () => {
      if (timer.current) clearInterval(timer.current);
    };
  }, [load]);

  const filtered = useMemo(() => {
    const r = RANGES.find((x) => x.key === range)!;
    if (!Number.isFinite(r.ms)) return logs;
    const cutoff = Date.now() - r.ms;
    return logs.filter((l) => new Date(l.ts).getTime() >= cutoff);
  }, [logs, range]);

  const shown = filtered.slice(0, PAGE_LIMIT);
  const capped = filtered.length > PAGE_LIMIT;

  return (
    <Shell>
      <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between mb-6">
        <div>
          <h1 className="text-xl font-semibold">调用日志</h1>
          <p className="text-sm text-[var(--muted-foreground)]">
            共 {filtered.length} 条{capped ? `，仅显示最近 ${PAGE_LIMIT} 条` : ""}，每 30 秒自动刷新
          </p>
        </div>
        <div className="flex gap-2 items-center flex-wrap">
          <div className="flex rounded-md border border-[var(--border)] overflow-hidden">
            {RANGES.map((r) => (
              <button
                key={r.key}
                onClick={() => setRange(r.key)}
                className={`px-3 py-1.5 text-xs transition-colors cursor-pointer ${
                  range === r.key
                    ? "bg-[var(--primary)] text-[var(--primary-foreground)]"
                    : "bg-transparent text-[var(--foreground)] hover:bg-[var(--accent)]"
                }`}
              >
                {r.label}
              </button>
            ))}
          </div>
          <Button variant="outline" size="sm" onClick={() => load()}>
            <RefreshCw className="h-4 w-4" />
            刷新
          </Button>
        </div>
      </div>

      <motion.div initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0 }} transition={{ duration: 0.3 }}>
        {loading ? (
          <Card>
            <CardContent className="pt-6 space-y-3">
              <Skeleton className="h-10 w-full" />
              <Skeleton className="h-10 w-full" />
              <Skeleton className="h-10 w-full" />
            </CardContent>
          </Card>
        ) : shown.length === 0 ? (
          <Card>
            <CardContent className="py-16 text-center text-sm text-[var(--muted-foreground)]">
              该时间范围内暂无调用记录
            </CardContent>
          </Card>
        ) : (
          <>
            {/* 桌面端：表格视图（md 及以上显示） */}
            <Card className="hidden md:block">
              <CardContent className="pt-6">
                <div className="max-h-[65vh] overflow-y-auto">
                  <Table>
                    <TableHeader>
                      <TableRow>
                        <TableHead>时间</TableHead>
                        <TableHead>模型</TableHead>
                        <TableHead>密钥</TableHead>
                        <TableHead>账号</TableHead>
                        <TableHead>解析层级</TableHead>
                        <TableHead>尝试</TableHead>
                        <TableHead>来源 IP</TableHead>
                        <TableHead>状态码</TableHead>
                        <TableHead>首字延迟</TableHead>
                        <TableHead>总耗时</TableHead>
                        <TableHead>Token</TableHead>
                        <TableHead>实际扣费</TableHead>
                        <TableHead>缓存命中</TableHead>
                      </TableRow>
                    </TableHeader>
                    <TableBody>
                      {shown.map((log, i) => (
                        <TableRow key={`${log.ts}-${i}`}>
                          <TableCell className="text-[var(--muted-foreground)] whitespace-nowrap">
                            {formatTime(log.ts)}
                          </TableCell>
                          <TableCell className="font-medium">{log.model || "-"}</TableCell>
                          <TableCell className="text-xs">{log.key_name || "-"}</TableCell>
                          <TableCell className="text-xs" title={log.retry_account_ids?.length ? `重试链：${log.retry_account_ids.join(" → ")}` : undefined}>
                            {log.account_id || "-"}
                          </TableCell>
                          <TableCell className="text-xs">
                            {log.route_via || "-"}
                            {log.downgrade_observed ? "（观察）" : ""}
                          </TableCell>
                          <TableCell className="text-xs">{log.attempts && log.attempts > 1 ? `${log.attempts}` : "1"}</TableCell>
                          <TableCell className="font-mono text-xs text-[var(--muted-foreground)]">
                            {log.source_ip || "-"}
                          </TableCell>
                          <TableCell>{statusBadge(log.status)}</TableCell>
                          <TableCell>{dash(log.ttft_ms && log.ttft_ms > 0 ? log.ttft_ms : undefined, formatLatency)}</TableCell>
                          <TableCell>{formatLatency(log.latency_ms)}</TableCell>
                          <TableCell>
                            {log.total_tokens ? `${log.total_tokens.toLocaleString()}` : "—"}
                          </TableCell>
                          <TableCell>{dash(log.credit)}</TableCell>
                          <TableCell>{dash(log.cached_tokens, (n) => n.toLocaleString())}</TableCell>
                        </TableRow>
                      ))}
                    </TableBody>
                  </Table>
                </div>
              </CardContent>
            </Card>

            {/* 移动端：纵向卡片视图（md 以下显示） */}
            <div className="md:hidden space-y-3 max-h-[70vh] overflow-y-auto">
              {shown.map((log, i) => (
                <LogCard key={`${log.ts}-${i}`} log={log} />
              ))}
            </div>
          </>
        )}
      </motion.div>
    </Shell>
  );
}
