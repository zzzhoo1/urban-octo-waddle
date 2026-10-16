"use client";

import { useCallback, useEffect, useState } from "react";
import { motion } from "motion/react";
import { toast } from "sonner";
import { KeyRound, Plus, RefreshCw, Trash2 } from "lucide-react";

import { api, type ApiKeyInfo } from "@/lib/api";
import { formatTime } from "@/lib/utils";
import { Shell } from "@/components/shell";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";

function keyStatusBadge(k: ApiKeyInfo) {
  if (k.expired) return <Badge variant="warning">已过期</Badge>;
  if (!k.enabled) return <Badge variant="secondary">已禁用</Badge>;
  if (k.token_quota > 0 && k.tokens_used >= k.token_quota)
    return <Badge variant="warning">配额用尽</Badge>;
  return <Badge variant="success">生效中</Badge>;
}

function quotaText(k: ApiKeyInfo): string {
  if (k.token_quota === 0) return `${k.tokens_used.toLocaleString()} / 不限`;
  const pct = Math.min(100, Math.round((k.tokens_used / k.token_quota) * 100));
  return `${k.tokens_used.toLocaleString()} / ${k.token_quota.toLocaleString()} (${pct}%)`;
}

/** 移动端纵向卡片：一把密钥一张卡 */
function KeyCard({
  k,
  onToggle,
  onDelete,
}: {
  k: ApiKeyInfo;
  onToggle: (k: ApiKeyInfo, enabled: boolean) => void;
  onDelete: (k: ApiKeyInfo) => void;
}) {
  return (
    <div className="rounded-lg border border-[var(--border)] bg-[var(--card)] p-3 space-y-1.5">
      <div className="flex items-center justify-between pb-1 border-b border-[var(--border)]">
        <span className="font-medium text-sm">{k.name}</span>
        {keyStatusBadge(k)}
      </div>
      <div className="flex items-center justify-between text-sm">
        <span className="text-xs text-[var(--muted-foreground)]">密钥</span>
        <span className="font-mono text-xs">{k.hint}</span>
      </div>
      <div className="flex items-center justify-between text-sm">
        <span className="text-xs text-[var(--muted-foreground)]">用量/配额</span>
        <span className="text-xs">{quotaText(k)}</span>
      </div>
      <div className="flex items-center justify-between text-sm">
        <span className="text-xs text-[var(--muted-foreground)]">模型白名单</span>
        <span className="text-xs">
          {k.model_allowlist.length === 0 ? "全部模型" : k.model_allowlist.join(", ")}
        </span>
      </div>
      <div className="flex items-center justify-between text-sm">
        <span className="text-xs text-[var(--muted-foreground)]">过期时间</span>
        <span className="text-xs">{k.expires_at ? formatTime(k.expires_at) : "永不过期"}</span>
      </div>
      <div className="flex items-center justify-between pt-1">
        <div className="flex items-center gap-2">
          <Switch checked={k.enabled} onCheckedChange={(c) => onToggle(k, c)} />
          <span className="text-xs text-[var(--muted-foreground)]">{k.enabled ? "已启用" : "已禁用"}</span>
        </div>
        <Button variant="ghost" size="icon" onClick={() => onDelete(k)}>
          <Trash2 className="h-4 w-4 text-[var(--destructive)]" />
        </Button>
      </div>
    </div>
  );
}

export default function KeysPage() {
  const [keys, setKeys] = useState<ApiKeyInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [addOpen, setAddOpen] = useState(false);
  const [name, setName] = useState("");
  const [keyValue, setKeyValue] = useState("");
  const [expiresAt, setExpiresAt] = useState("");
  const [quota, setQuota] = useState("");
  const [allowlist, setAllowlist] = useState("");
  const [saving, setSaving] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<ApiKeyInfo | null>(null);
  const [deleting, setDeleting] = useState(false);

  const load = useCallback(async () => {
    try {
      setKeys(await api.listKeys());
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

  const randomKey = () => {
    const bytes = new Uint8Array(24);
    crypto.getRandomValues(bytes);
    const s = Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
    setKeyValue(`sk-${s}`);
  };

  const createKey = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!name.trim() || !keyValue.trim()) {
      toast.error("名称和密钥都不能为空");
      return;
    }
    setSaving(true);
    try {
      await api.createKey({
        name: name.trim(),
        key: keyValue.trim(),
        expires_at: expiresAt ? new Date(expiresAt).toISOString() : null,
        token_quota: quota ? parseInt(quota, 10) : 0,
        model_allowlist: allowlist
          .split(",")
          .map((s) => s.trim())
          .filter(Boolean),
      });
      toast.success(`密钥『${name.trim()}』已创建，请立即复制保存完整密钥`);
      setAddOpen(false);
      setName("");
      setKeyValue("");
      setExpiresAt("");
      setQuota("");
      setAllowlist("");
      await load();
    } catch (err) {
      toast.error(`创建失败：${err instanceof Error ? err.message : "未知错误"}`);
    } finally {
      setSaving(false);
    }
  };

  const toggle = async (k: ApiKeyInfo, enabled: boolean) => {
    try {
      await api.patchKey(k.name, { enabled });
      toast.success(`密钥『${k.name}』已${enabled ? "启用" : "禁用"}`);
      await load();
    } catch (err) {
      toast.error(`操作失败：${err instanceof Error ? err.message : "未知错误"}`);
    }
  };

  const remove = async () => {
    if (!deleteTarget) return;
    setDeleting(true);
    try {
      await api.deleteKey(deleteTarget.name);
      toast.success(`密钥『${deleteTarget.name}』已删除`);
      setDeleteTarget(null);
      await load();
    } catch (err) {
      toast.error(`删除失败：${err instanceof Error ? err.message : "未知错误"}`);
    } finally {
      setDeleting(false);
    }
  };

  return (
    <Shell>
      <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between mb-6">
        <div>
          <h1 className="text-xl font-semibold">密钥分发</h1>
          <p className="text-sm text-[var(--muted-foreground)]">
            共 {keys.length} 把密钥，各自独立配额与模型白名单
          </p>
        </div>
        <div className="flex gap-2">
          <Button variant="outline" size="sm" onClick={load}>
            <RefreshCw className="h-4 w-4" />
            刷新
          </Button>
          <Button size="sm" onClick={() => setAddOpen(true)}>
            <Plus className="h-4 w-4" />
            新建密钥
          </Button>
        </div>
      </div>

      <motion.div initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0 }} transition={{ duration: 0.3 }}>
        {loading ? (
          <Card>
            <CardContent className="pt-6 space-y-3">
              <Skeleton className="h-10 w-full" />
              <Skeleton className="h-10 w-full" />
            </CardContent>
          </Card>
        ) : keys.length === 0 ? (
          <Card>
            <CardContent className="py-16 text-center text-sm text-[var(--muted-foreground)]">
              暂无密钥，点击右上角『新建密钥』创建第一把
            </CardContent>
          </Card>
        ) : (
          <>
            {/* 桌面端：表格视图 */}
            <Card className="hidden md:block">
              <CardContent className="pt-6">
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>名称</TableHead>
                      <TableHead>密钥</TableHead>
                      <TableHead>状态</TableHead>
                      <TableHead>用量 / 配额</TableHead>
                      <TableHead>模型白名单</TableHead>
                      <TableHead>过期时间</TableHead>
                      <TableHead>启用</TableHead>
                      <TableHead className="text-right">操作</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {keys.map((k) => (
                      <TableRow key={k.name}>
                        <TableCell className="font-medium">{k.name}</TableCell>
                        <TableCell className="font-mono text-xs text-[var(--muted-foreground)]">
                          {k.hint}
                        </TableCell>
                        <TableCell>{keyStatusBadge(k)}</TableCell>
                        <TableCell className="text-sm">{quotaText(k)}</TableCell>
                        <TableCell className="text-xs text-[var(--muted-foreground)]">
                          {k.model_allowlist.length === 0
                            ? "全部模型"
                            : k.model_allowlist.join(", ")}
                        </TableCell>
                        <TableCell className="text-[var(--muted-foreground)]">
                          {k.expires_at ? formatTime(k.expires_at) : "永不过期"}
                        </TableCell>
                        <TableCell>
                          <Switch
                            checked={k.enabled}
                            onCheckedChange={(checked) => toggle(k, checked)}
                          />
                        </TableCell>
                        <TableCell className="text-right">
                          <Button
                            variant="ghost"
                            size="icon"
                            onClick={() => setDeleteTarget(k)}
                          >
                            <Trash2 className="h-4 w-4 text-[var(--destructive)]" />
                          </Button>
                        </TableCell>
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              </CardContent>
            </Card>

            {/* 移动端：纵向卡片视图 */}
            <div className="md:hidden space-y-3">
              {keys.map((k) => (
                <KeyCard key={k.name} k={k} onToggle={toggle} onDelete={setDeleteTarget} />
              ))}
            </div>
          </>
        )}
      </motion.div>

      <Dialog open={addOpen} onOpenChange={setAddOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>新建密钥</DialogTitle>
            <DialogDescription>
              完整密钥仅在创建时可见一次，此后只显示掩码；请立即复制保存
            </DialogDescription>
          </DialogHeader>
          <form onSubmit={createKey} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="key-name">密钥名称</Label>
              <Input
                id="key-name"
                placeholder="例如 team-frontend"
                value={name}
                onChange={(e) => setName(e.target.value)}
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="key-value">密钥内容</Label>
              <div className="flex gap-2">
                <Input
                  id="key-value"
                  placeholder="sk-..."
                  value={keyValue}
                  onChange={(e) => setKeyValue(e.target.value)}
                  className="font-mono text-xs"
                />
                <Button type="button" variant="outline" onClick={randomKey}>
                  <KeyRound className="h-4 w-4" />
                  生成
                </Button>
              </div>
            </div>
            <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
              <div className="space-y-2">
                <Label htmlFor="key-expires">过期时间（可选）</Label>
                <Input
                  id="key-expires"
                  type="datetime-local"
                  value={expiresAt}
                  onChange={(e) => setExpiresAt(e.target.value)}
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="key-quota">Token 配额（0 = 不限）</Label>
                <Input
                  id="key-quota"
                  type="number"
                  min="0"
                  placeholder="0"
                  value={quota}
                  onChange={(e) => setQuota(e.target.value)}
                />
              </div>
            </div>
            <div className="space-y-2">
              <Label htmlFor="key-allow">模型白名单（逗号分隔，留空 = 全部）</Label>
              <Input
                id="key-allow"
                placeholder="auto, fast-model"
                value={allowlist}
                onChange={(e) => setAllowlist(e.target.value)}
              />
            </div>
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setAddOpen(false)}>
                取消
              </Button>
              <Button type="submit" disabled={saving}>
                {saving ? "创建中..." : "创建"}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      <Dialog open={!!deleteTarget} onOpenChange={() => setDeleteTarget(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>确认删除密钥</DialogTitle>
            <DialogDescription>
              即将删除密钥『{deleteTarget?.name}』，使用该密钥的调用会立即被拒绝
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteTarget(null)}>
              取消
            </Button>
            <Button variant="destructive" onClick={remove} disabled={deleting}>
              {deleting ? "删除中..." : "确认删除"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </Shell>
  );
}
