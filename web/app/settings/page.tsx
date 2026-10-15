"use client";

import { useCallback, useEffect, useState } from "react";
import { motion } from "motion/react";
import { toast } from "sonner";
import { Info, RefreshCw, Server } from "lucide-react";

import { api, type GatewayInfo } from "@/lib/api";
import { Shell } from "@/components/shell";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";

export default function SettingsPage() {
  const [info, setInfo] = useState<GatewayInfo | null>(null);
  const [loading, setLoading] = useState(true);

  const load = useCallback(async () => {
    try {
      setInfo(await api.info());
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

  const rows: Array<[string, React.ReactNode]> = info
    ? [
        ["版本", <Badge key="v" variant="secondary">v{info.version}</Badge>],
        ["监听地址", info.listen],
        ["上游网关", <span key="u" className="font-mono text-xs">{info.upstream}</span>],
        ["默认模型", info.default_model],
        ["熔断阈值", `${info.circuit_threshold} 次连续失败`],
        ["熔断冷却", `${info.circuit_cooldown_secs} 秒`],
        ["账号失败阈值", `${info.failure_threshold} 次`],
        ["账号冷却恢复", `${info.account_recover_after_secs} 秒自动恢复`],
        ["最大重试", `${info.retry_max} 次（线性退避 ${info.retry_backoff_ms}ms 起）`],
        ["会话粘性", `TTL ${info.session_ttl_secs / 60} 分钟，容量 ${info.session_max_size}`],
        ["当前粘性会话", `${info.sticky_sessions} 个`],
        ["API 密钥", `${info.api_keys} 把`],
      ]
    : [];

  return (
    <Shell>
      <div className="flex items-center justify-between mb-6">
        <div>
          <h1 className="text-xl font-semibold">系统设置</h1>
          <p className="text-sm text-[var(--muted-foreground)]">
            网关运行配置为只读，修改需调整启动参数后重启服务
          </p>
        </div>
        <Button variant="outline" size="sm" onClick={load}>
          <RefreshCw className="h-4 w-4" />
          刷新
        </Button>
      </div>

      <div className="grid gap-4 lg:grid-cols-2">
        <motion.div initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0 }} transition={{ duration: 0.3 }}>
          <Card>
            <CardHeader>
              <CardTitle className="flex items-center gap-2">
                <Server className="h-4 w-4 text-[var(--primary)]" />
                运行配置
              </CardTitle>
              <CardDescription>当前生效的网关参数</CardDescription>
            </CardHeader>
            <CardContent>
              {loading ? (
                <div className="space-y-3">
                  <Skeleton className="h-8 w-full" />
                  <Skeleton className="h-8 w-full" />
                  <Skeleton className="h-8 w-full" />
                </div>
              ) : (
                <dl className="divide-y divide-[var(--border)]">
                  {rows.map(([label, value]) => (
                    <div key={label} className="flex items-center justify-between py-2.5">
                      <dt className="text-sm text-[var(--muted-foreground)]">{label}</dt>
                      <dd className="text-sm font-medium">{value}</dd>
                    </div>
                  ))}
                </dl>
              )}
            </CardContent>
          </Card>
        </motion.div>

        <motion.div
          initial={{ opacity: 0, y: 12 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ delay: 0.05, duration: 0.3 }}
        >
          <Card>
            <CardHeader>
              <CardTitle className="flex items-center gap-2">
                <Info className="h-4 w-4 text-[var(--primary)]" />
                部署说明
              </CardTitle>
              <CardDescription>如何更新网关版本</CardDescription>
            </CardHeader>
            <CardContent className="text-sm space-y-3 text-[var(--foreground)]">
              <ol className="list-decimal list-inside space-y-2">
                <li>在 GitHub 仓库推送 tag，流水线自动完成前端构建与 Rust 编译</li>
                <li>从 Release 下载 codearts-gateway 二进制及 sha256 校验文件</li>
                <li>核对摘要后替换服务器上的旧二进制</li>
                <li>重启服务即完成升级，账号数据（accounts.json）保持兼容</li>
              </ol>
              <div className="rounded-lg border border-[var(--border)] bg-[var(--muted)] p-3 text-xs text-[var(--muted-foreground)]">
                提示：前端已静态内嵌进二进制，无需单独部署 Node 环境；
                启动参数与旧版完全一致（--listen / --secret / --store / --codearts-ak / --codearts-sk / --gw-key）。
              </div>
            </CardContent>
          </Card>
        </motion.div>
      </div>
    </Shell>
  );
}
