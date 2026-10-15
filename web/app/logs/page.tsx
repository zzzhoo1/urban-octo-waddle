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
  { key: "all", label: "全部", ms: Infinity },
] as const;

const PAGE_LIMIT = 200;

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
      <div className="flex items-center justify-between mb-6">
        <div>
          <h1 className="text-xl font-semibold">调用日志</h1>
          <p className="text-sm text-[var(--muted-foreground)]">
            共 {filtered.length} 条{capped ? `，仅显示最近 ${PAGE_LIMIT} 条` : ""}，每 30 秒自动刷新
          </p>
        </div>
        <div className="flex gap-2 items-center">
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
        <Card>
          <CardContent className="pt-6">
            {loading ? (
              <div className="space-y-3">
                <Skeleton className="h-10 w-full" />
                <Skeleton className="h-10 w-full" />
                <Skeleton className="h-10 w-full" />
              </div>
            ) : shown.length === 0 ? (
              <div className="py-16 text-center text-sm text-[var(--muted-foreground)]">
                该时间范围内暂无调用记录
              </div>
            ) : (
              <div className="max-h-[65vh] overflow-y-auto">
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>时间</TableHead>
                      <TableHead>模型</TableHead>
                      <TableHead>密钥</TableHead>
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
                        <TableCell className="font-mono text-xs text-[var(--muted-foreground)]">
                          {log.source_ip || "-"}
                        </TableCell>
                        <TableCell>{statusBadge(log.status)}</TableCell>
                        <TableCell>{dash(log.ttft_ms && log.ttft_ms > 0 ? log.ttft_ms : undefined, formatLatency)}</TableCell>
                        <TableCell>{formatLatency(log.latency_ms)}</TableCell>
                        <TableCell>
                          {log.total_tokens
                            ? `${log.total_tokens.toLocaleString()}`
                            : "—"}
                        </TableCell>
                        <TableCell>{dash(log.credit)}</TableCell>
                        <TableCell>{dash(log.cached_tokens, (n) => n.toLocaleString())}</TableCell>
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              </div>
            )}
          </CardContent>
        </Card>
      </motion.div>
    </Shell>
  );
}
