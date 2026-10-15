"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { motion } from "motion/react";
import { toast } from "sonner";
import { ArrowRight, Boxes, RefreshCw, Sparkles } from "lucide-react";

import { listModels, type ModelEntry } from "@/lib/api";
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

// Static tier map (mirrors the gateway's built-in alias table).
const TIER_MAP: Record<string, { model: string; effort?: string; desc: string }> = {
  auto: { model: "GLM-5.2", desc: "自动选择，默认主模型" },
  "fast-model": { model: "openpangu-2.0-flash", desc: "快速响应档位" },
  "balanced-model": { model: "openpangu-2.0-pro", desc: "均衡档位" },
  "primary-model": { model: "GLM-5.2", desc: "主模型档位" },
  "deep-model": { model: "GLM-5.2", effort: "max", desc: "深度思考档位" },
};

export default function ModelsPage() {
  const [models, setModels] = useState<ModelEntry[]>([]);
  const [loading, setLoading] = useState(true);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      setModels(await listModels());
    } catch (e) {
      toast.error(`加载模型列表失败：${e instanceof Error ? e.message : "未知错误"}`);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const tiers = useMemo(() => models.filter((m) => m.owned_by === "codearts-gateway"), [models]);
  const upstream = useMemo(
    () => models.filter((m) => m.owned_by === "huaweicloud-maas"),
    [models]
  );

  return (
    <Shell>
      <div className="flex items-center justify-between mb-6">
        <div>
          <h1 className="text-xl font-semibold">模型中心</h1>
          <p className="text-sm text-[var(--muted-foreground)]">
            档位别名由网关解析，上游模型清单每 10 分钟动态刷新
          </p>
        </div>
        <Button variant="outline" size="sm" onClick={load}>
          <RefreshCw className="h-4 w-4" />
          刷新
        </Button>
      </div>

      <div className="grid gap-4 lg:grid-cols-2">
        <motion.div
          initial={{ opacity: 0, y: 12 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.3 }}
        >
          <Card>
            <CardHeader>
              <CardTitle className="flex items-center gap-2">
                <Sparkles className="h-4 w-4 text-[var(--primary)]" />
                档位别名（{tiers.length}）
              </CardTitle>
              <CardDescription>客户端可直接使用，网关自动映射到实际模型</CardDescription>
            </CardHeader>
            <CardContent className="space-y-3">
              {loading ? (
                <>
                  <Skeleton className="h-12 w-full" />
                  <Skeleton className="h-12 w-full" />
                  <Skeleton className="h-12 w-full" />
                </>
              ) : tiers.length === 0 ? (
                <div className="py-8 text-center text-sm text-[var(--muted-foreground)]">
                  未获取到档位列表
                </div>
              ) : (
                tiers.map((t) => {
                  const meta = TIER_MAP[t.id];
                  return (
                    <div
                      key={t.id}
                      className="flex items-center justify-between rounded-lg border border-[var(--border)] px-4 py-3"
                    >
                      <div>
                        <div className="font-medium text-sm">{t.id}</div>
                        <div className="text-xs text-[var(--muted-foreground)]">
                          {meta?.desc ?? "档位别名"}
                        </div>
                      </div>
                      <div className="flex items-center gap-2 text-sm">
                        <ArrowRight className="h-3.5 w-3.5 text-[var(--muted-foreground)]" />
                        <Badge variant="secondary">{meta?.model ?? "-"}</Badge>
                        {meta?.effort && <Badge variant="outline">effort: {meta.effort}</Badge>}
                      </div>
                    </div>
                  );
                })
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
                <Boxes className="h-4 w-4 text-[var(--primary)]" />
                上游可用模型（{upstream.length}）
              </CardTitle>
              <CardDescription>来自华为云 MaaS，按账号开通状态动态拉取</CardDescription>
            </CardHeader>
            <CardContent className="space-y-3">
              {loading ? (
                <>
                  <Skeleton className="h-12 w-full" />
                  <Skeleton className="h-12 w-full" />
                  <Skeleton className="h-12 w-full" />
                </>
              ) : upstream.length === 0 ? (
                <div className="py-8 text-center text-sm text-[var(--muted-foreground)]">
                  未获取到上游模型
                </div>
              ) : (
                upstream.map((m) => (
                  <div
                    key={m.id}
                    className="flex items-center justify-between rounded-lg border border-[var(--border)] px-4 py-3"
                  >
                    <div className="font-medium text-sm">{m.id}</div>
                    <Badge variant="outline">{m.owned_by}</Badge>
                  </div>
                ))
              )}
            </CardContent>
          </Card>
        </motion.div>
      </div>
    </Shell>
  );
}
