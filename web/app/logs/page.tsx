"use client";

import { useCallback, useEffect, useState } from "react";
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

export default function LogsPage() {
  const [logs, setLogs] = useState<RequestLog[]>([]);
  const [loading, setLoading] = useState(true);

  const load = useCallback(async () => {
    try {
      setLogs(await api.logs());
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

  return (
    <Shell>
      <div className="flex items-center justify-between mb-6">
        <div>
          <h1 className="text-xl font-semibold">调用日志</h1>
          <p className="text-sm text-[var(--muted-foreground)]">
            最近 {logs.length} 次数据面请求（内存环形缓冲，重启后清空）
          </p>
        </div>
        <Button variant="outline" size="sm" onClick={load}>
          <RefreshCw className="h-4 w-4" />
          刷新
        </Button>
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
            ) : logs.length === 0 ? (
              <div className="py-16 text-center text-sm text-[var(--muted-foreground)]">
                暂无调用记录
              </div>
            ) : (
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>时间</TableHead>
                    <TableHead>方法</TableHead>
                    <TableHead>路径</TableHead>
                    <TableHead>模型</TableHead>
                    <TableHead>状态码</TableHead>
                    <TableHead>耗时</TableHead>
                    <TableHead>会话</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {logs.map((log, i) => (
                    <TableRow key={`${log.ts}-${i}`}>
                      <TableCell className="text-[var(--muted-foreground)] whitespace-nowrap">
                        {formatTime(log.ts)}
                      </TableCell>
                      <TableCell>
                        <Badge variant="outline">{log.method}</Badge>
                      </TableCell>
                      <TableCell className="font-mono text-xs">{log.path}</TableCell>
                      <TableCell className="font-medium">{log.model || "-"}</TableCell>
                      <TableCell>{statusBadge(log.status)}</TableCell>
                      <TableCell>{formatLatency(log.latency_ms)}</TableCell>
                      <TableCell className="text-[var(--muted-foreground)] font-mono text-xs">
                        {log.session_id ? log.session_id.slice(0, 12) : "-"}
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            )}
          </CardContent>
        </Card>
      </motion.div>
    </Shell>
  );
}
