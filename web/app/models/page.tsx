"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { motion } from "motion/react";
import { toast } from "sonner";
import { ArrowRight, Boxes, Plus, RefreshCw, Save, Sparkles, Trash2 } from "lucide-react";

import { api, listModels, type ModelEntry, type RouteRule, type RouterConfig } from "@/lib/api";
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
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";

// Static tier map (mirrors the gateway's built-in alias table).
const TIER_MAP: Record<string, { model: string; effort?: string; desc: string }> = {
  auto: { model: "GLM-5.2", desc: "自动选择，默认主模型" },
  "fast-model": { model: "openpangu-2.0-flash", desc: "快速响应档位" },
  "balanced-model": { model: "openpangu-2.0-pro", desc: "均衡档位" },
  "primary-model": { model: "GLM-5.2", desc: "主模型档位" },
  "deep-model": { model: "GLM-5.2", effort: "max", desc: "深度思考档位" },
};

interface EditableRule extends RouteRule {
  _id: number;
}

export default function ModelsPage() {
  const [models, setModels] = useState<ModelEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [rules, setRules] = useState<EditableRule[]>([]);
  const [downgradeEnabled, setDowngradeEnabled] = useState(false);
  const [downgradeSources, setDowngradeSources] = useState("");
  const [downgradeTarget, setDowngradeTarget] = useState("");
  const [useHeuristic, setUseHeuristic] = useState(false);
  const [alsoDowngradeEffort, setAlsoDowngradeEffort] = useState(false);
  const [logOnly, setLogOnly] = useState(false);
  const [saving, setSaving] = useState(false);
  const [nextId, setNextId] = useState(1);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [m, rc] = await Promise.all([listModels(), api.getRouter()]);
      setModels(m);
      const editable = rc.rules.map((r, i) => ({ ...r, _id: i + 1 }));
      setRules(editable);
      setNextId(editable.length + 1);
      if (rc.downgrade) {
        setDowngradeEnabled(rc.downgrade.enabled);
        setDowngradeSources(rc.downgrade.sources.join(", "));
        setDowngradeTarget(rc.downgrade.target);
        setUseHeuristic(rc.downgrade.use_heuristic ?? false);
        setAlsoDowngradeEffort(rc.downgrade.also_downgrade_effort ?? false);
        setLogOnly(rc.downgrade.log_only ?? false);
      }
    } catch (e) {
      toast.error(`加载失败：${e instanceof Error ? e.message : "未知错误"}`);
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

  const addRule = () => {
    setRules([...rules, { _id: nextId, alias: "", pattern: "", target: "", effort: "", note: "" }]);
    setNextId(nextId + 1);
  };

  const updateRule = (id: number, field: keyof RouteRule, value: string) => {
    setRules(rules.map((r) => (r._id === id ? { ...r, [field]: value } : r)));
  };

  const removeRule = (id: number) => {
    setRules(rules.filter((r) => r._id !== id));
  };

  const save = async () => {
    // Validate: each rule needs alias XOR pattern, and a target.
    for (const r of rules) {
      const hasAlias = !!r.alias?.trim();
      const hasPattern = !!r.pattern?.trim();
      if (hasAlias === hasPattern) {
        toast.error("每条规则必须且只能填『别名』或『匹配模式』其中一项");
        return;
      }
      if (!r.target.trim()) {
        toast.error("每条规则必须填写目标模型");
        return;
      }
    }
    setSaving(true);
    try {
      const payload: RouterConfig = {
        rules: rules.map((r) => ({
          alias: r.alias?.trim() || undefined,
          pattern: r.pattern?.trim() || undefined,
          target: r.target.trim(),
          effort: r.effort?.trim() || undefined,
          note: r.note?.trim() || undefined,
        })),
        downgrade: downgradeEnabled
          ? {
              sources: downgradeSources
                .split(",")
                .map((s) => s.trim())
                .filter(Boolean),
              target: downgradeTarget.trim(),
              enabled: true,
              use_heuristic: useHeuristic,
              also_downgrade_effort: alsoDowngradeEffort,
              log_only: logOnly,
            }
          : null,
      };
      await api.putRouter(payload);
      toast.success("路由规则已保存并即时生效");
      await load();
    } catch (err) {
      toast.error(`保存失败：${err instanceof Error ? err.message : "未知错误"}`);
    } finally {
      setSaving(false);
    }
  };

  return (
    <Shell>
      <div className="flex items-center justify-between mb-6">
        <div>
          <h1 className="text-xl font-semibold">模型中心</h1>
          <p className="text-sm text-[var(--muted-foreground)]">
            自定义规则优先于内置档位；保存后立即生效，无需重启
          </p>
        </div>
        <div className="flex gap-2">
          <Button variant="outline" size="sm" onClick={load}>
            <RefreshCw className="h-4 w-4" />
            刷新
          </Button>
          <Button size="sm" onClick={save} disabled={saving}>
            <Save className="h-4 w-4" />
            {saving ? "保存中..." : "保存规则"}
          </Button>
        </div>
      </div>

      <div className="grid gap-4 lg:grid-cols-2 mb-6">
        <motion.div initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0 }} transition={{ duration: 0.3 }}>
          <Card>
            <CardHeader>
              <CardTitle className="flex items-center gap-2">
                <Sparkles className="h-4 w-4 text-[var(--primary)]" />
                内置档位（{tiers.length}）
              </CardTitle>
              <CardDescription>网关自带的别名，自定义规则可覆盖</CardDescription>
            </CardHeader>
            <CardContent className="space-y-3">
              {loading ? (
                <Skeleton className="h-24 w-full" />
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

        <motion.div initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0 }} transition={{ delay: 0.05, duration: 0.3 }}>
          <Card>
            <CardHeader>
              <CardTitle className="flex items-center gap-2">
                <Boxes className="h-4 w-4 text-[var(--primary)]" />
                上游可用模型（{upstream.length}）
              </CardTitle>
              <CardDescription>路由目标从这些模型中选择</CardDescription>
            </CardHeader>
            <CardContent className="space-y-3">
              {loading ? (
                <Skeleton className="h-24 w-full" />
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

      <motion.div initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0 }} transition={{ delay: 0.1, duration: 0.3 }}>
        <Card className="mb-6">
          <CardHeader>
            <div className="flex items-center justify-between">
              <div>
                <CardTitle>自定义路由规则</CardTitle>
                <CardDescription>
                  按列表顺序匹配，第一条命中即生效；匹配模式支持 * ? {a,b}（glob 语法）
                </CardDescription>
              </div>
              <Button variant="outline" size="sm" onClick={addRule}>
                <Plus className="h-4 w-4" />
                添加规则
              </Button>
            </div>
          </CardHeader>
          <CardContent className="space-y-3">
            {rules.length === 0 ? (
              <div className="py-8 text-center text-sm text-[var(--muted-foreground)]">
                暂无自定义规则，点击『添加规则』创建别名映射
              </div>
            ) : (
              rules.map((r) => (
                <div
                  key={r._id}
                  className="grid grid-cols-12 gap-2 items-center rounded-lg border border-[var(--border)] p-3"
                >
                  <div className="col-span-3">
                    <Label className="text-xs">别名（精确匹配）</Label>
                    <Input
                      placeholder="my-model"
                      value={r.alias ?? ""}
                      onChange={(e) => updateRule(r._id, "alias", e.target.value)}
                    />
                  </div>
                  <div className="col-span-3">
                    <Label className="text-xs">匹配模式（二选一）</Label>
                    <Input
                      placeholder="^gpt-.*"
                      value={r.pattern ?? ""}
                      onChange={(e) => updateRule(r._id, "pattern", e.target.value)}
                    />
                  </div>
                  <div className="col-span-2">
                    <Label className="text-xs">目标模型</Label>
                    <Input
                      placeholder="GLM-5.2"
                      value={r.target}
                      onChange={(e) => updateRule(r._id, "target", e.target.value)}
                    />
                  </div>
                  <div className="col-span-2">
                    <Label className="text-xs">思考档位（可选）</Label>
                    <Input
                      placeholder="high / max"
                      value={r.effort ?? ""}
                      onChange={(e) => updateRule(r._id, "effort", e.target.value)}
                    />
                  </div>
                  <div className="col-span-2 flex items-end justify-end">
                    <Button variant="ghost" size="icon" onClick={() => removeRule(r._id)}>
                      <Trash2 className="h-4 w-4 text-[var(--destructive)]" />
                    </Button>
                  </div>
                </div>
              ))
            )}
          </CardContent>
        </Card>
      </motion.div>

      <motion.div initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0 }} transition={{ delay: 0.15, duration: 0.3 }}>
        <Card>
          <CardHeader>
            <div className="flex items-center justify-between">
              <div>
                <CardTitle>后台任务自动降级</CardTitle>
                <CardDescription>
                  后台请求自动改用到便宜模型，为前台流量保留优质配额；降级对客户端透明，响应中的模型名仍为客户端传入名，可在调用日志中通过解析层级确认
                </CardDescription>
              </div>
              <Switch checked={downgradeEnabled} onCheckedChange={setDowngradeEnabled} />
            </div>
          </CardHeader>
          {downgradeEnabled && (
            <CardContent className="space-y-4">
              <div className="grid grid-cols-2 gap-4">
                <div className="space-y-2">
                  <Label>降级源模型（逗号分隔）</Label>
                  <Input
                    placeholder="GLM-5.2, kimi-k2.6-vl"
                    value={downgradeSources}
                    onChange={(e) => setDowngradeSources(e.target.value)}
                  />
                </div>
                <div className="space-y-2">
                  <Label>降级目标模型</Label>
                  <Input
                    placeholder="openpangu-2.0-flash"
                    value={downgradeTarget}
                    onChange={(e) => setDowngradeTarget(e.target.value)}
                  />
                </div>
              </div>
              <div className="space-y-3 rounded-md border border-[var(--border)] p-3">
                <div className="flex items-center justify-between">
                  <div className="leading-tight">
                    <p className="text-sm font-medium">启发式后台判定</p>
                    <p className="text-xs text-[var(--muted-foreground)]">
                      请求头与 background 字段均未标记时，按信号判定（tools+tool 角色历史、bg- 前缀 user 字段，命中 2 个信号才算后台）
                    </p>
                  </div>
                  <Switch checked={useHeuristic} onCheckedChange={setUseHeuristic} />
                </div>
                <div className="flex items-center justify-between">
                  <div className="leading-tight">
                    <p className="text-sm font-medium">同时降级思考档位</p>
                    <p className="text-xs text-[var(--muted-foreground)]">
                      降级时把 reasoning_effort 压到 low
                    </p>
                  </div>
                  <Switch checked={alsoDowngradeEffort} onCheckedChange={setAlsoDowngradeEffort} />
                </div>
                <div className="flex items-center justify-between">
                  <div className="leading-tight">
                    <p className="text-sm font-medium">仅观察模式（log_only）</p>
                    <p className="text-xs text-[var(--muted-foreground)]">
                      只记录『本应降级』而不实际改写模型，用于灰度验证启发式准确性
                    </p>
                  </div>
                  <Switch checked={logOnly} onCheckedChange={setLogOnly} />
                </div>
              </div>
            </CardContent>
          )}
        </Card>
      </motion.div>
    </Shell>
  );
}
