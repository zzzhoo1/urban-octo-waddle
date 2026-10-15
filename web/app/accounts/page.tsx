"use client";

import { useCallback, useEffect, useState } from "react";
import { motion } from "motion/react";
import { toast } from "sonner";
import { Plus, RefreshCw, Trash2 } from "lucide-react";

import { api, type Account } from "@/lib/api";
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

function statusBadge(status: string) {
  switch (status) {
    case "active":
      return <Badge variant="success">运行中</Badge>;
    case "cooling":
      return <Badge variant="warning">冷却中</Badge>;
    case "disabled":
      return <Badge variant="secondary">已禁用</Badge>;
    default:
      return <Badge variant="outline">{status}</Badge>;
  }
}

export default function AccountsPage() {
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [loading, setLoading] = useState(true);
  const [addOpen, setAddOpen] = useState(false);
  const [name, setName] = useState("");
  const [credential, setCredential] = useState("");
  const [saving, setSaving] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<Account | null>(null);
  const [deleting, setDeleting] = useState(false);

  const load = useCallback(async () => {
    try {
      setAccounts(await api.listAccounts());
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

  const createAccount = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!name.trim() || !credential.trim()) {
      toast.error("名称和凭据都不能为空");
      return;
    }
    setSaving(true);
    try {
      await api.createAccount(name.trim(), credential.trim());
      toast.success(`账号『${name.trim()}』已添加`);
      setAddOpen(false);
      setName("");
      setCredential("");
      await load();
    } catch (err) {
      toast.error(`添加失败：${err instanceof Error ? err.message : "未知错误"}`);
    } finally {
      setSaving(false);
    }
  };

  const toggle = async (acc: Account, enabled: boolean) => {
    try {
      await api.patchAccount(acc.id, enabled ? "active" : "disabled");
      toast.success(`账号『${acc.name}』已${enabled ? "启用" : "禁用"}`);
      await load();
    } catch (err) {
      toast.error(`操作失败：${err instanceof Error ? err.message : "未知错误"}`);
    }
  };

  const remove = async () => {
    if (!deleteTarget) return;
    setDeleting(true);
    try {
      await api.deleteAccount(deleteTarget.id);
      toast.success(`账号『${deleteTarget.name}』已删除`);
      setDeleteTarget(null);
      await load();
    } catch (err) {
      toast.error(`删除失败：${err instanceof Error ? err.message : "未知错误"}`);
    } finally {
      setDeleting(false);
    }
  };

  const oauthAdd = async () => {
    try {
      const r = await api.oauthStart();
      window.open(r.auth_url, "_blank");
    } catch (err) {
      toast.error(
        err instanceof Error && err.message.includes("501")
          ? "当前后端未配置 OAuth，请使用手动录入"
          : `OAuth 启动失败：${err instanceof Error ? err.message : "未知错误"}`
      );
    }
  };

  return (
    <Shell>
      <div className="flex items-center justify-between mb-6">
        <div>
          <h1 className="text-xl font-semibold">账号管理</h1>
          <p className="text-sm text-[var(--muted-foreground)]">
            共 {accounts.length} 个账号，凭据加密存储，页面不展示明文
          </p>
        </div>
        <div className="flex gap-2">
          <Button variant="outline" size="sm" onClick={load}>
            <RefreshCw className="h-4 w-4" />
            刷新
          </Button>
          <Button variant="outline" size="sm" onClick={oauthAdd}>
            OAuth 添加
          </Button>
          <Button size="sm" onClick={() => setAddOpen(true)}>
            <Plus className="h-4 w-4" />
            手动录入
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
            ) : accounts.length === 0 ? (
              <div className="py-16 text-center text-sm text-[var(--muted-foreground)]">
                暂无账号，点击右上角『手动录入』添加第一个账号
              </div>
            ) : (
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>名称</TableHead>
                    <TableHead>状态</TableHead>
                    <TableHead>失败计数</TableHead>
                    <TableHead>最近失败</TableHead>
                    <TableHead>创建时间</TableHead>
                    <TableHead>启用</TableHead>
                    <TableHead className="text-right">操作</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {accounts.map((acc) => (
                    <TableRow key={acc.id}>
                      <TableCell className="font-medium">{acc.name}</TableCell>
                      <TableCell>{statusBadge(acc.status)}</TableCell>
                      <TableCell>
                        {acc.failure_count > 0 ? (
                          <span className="text-amber-500 font-medium">{acc.failure_count}</span>
                        ) : (
                          <span className="text-[var(--muted-foreground)]">0</span>
                        )}
                      </TableCell>
                      <TableCell className="text-[var(--muted-foreground)]">
                        {formatTime(acc.last_failure_at)}
                      </TableCell>
                      <TableCell className="text-[var(--muted-foreground)]">
                        {formatTime(acc.created_at)}
                      </TableCell>
                      <TableCell>
                        <Switch
                          checked={acc.status !== "disabled"}
                          onCheckedChange={(checked) => toggle(acc, checked)}
                        />
                      </TableCell>
                      <TableCell className="text-right">
                        <Button
                          variant="ghost"
                          size="icon"
                          onClick={() => setDeleteTarget(acc)}
                        >
                          <Trash2 className="h-4 w-4 text-[var(--destructive)]" />
                        </Button>
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            )}
          </CardContent>
        </Card>
      </motion.div>

      <Dialog open={addOpen} onOpenChange={setAddOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>手动录入账号</DialogTitle>
            <DialogDescription>
              凭据将使用 AES-256-GCM 加密后存储，任何接口都不会返回明文
            </DialogDescription>
          </DialogHeader>
          <form onSubmit={createAccount} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="acc-name">账号名称</Label>
              <Input
                id="acc-name"
                placeholder="例如 codearts-backup"
                value={name}
                onChange={(e) => setName(e.target.value)}
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="acc-cred">凭据（Access Token）</Label>
              <Input
                id="acc-cred"
                type="password"
                placeholder="粘贴上游凭据"
                value={credential}
                onChange={(e) => setCredential(e.target.value)}
              />
            </div>
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setAddOpen(false)}>
                取消
              </Button>
              <Button type="submit" disabled={saving}>
                {saving ? "保存中..." : "保存"}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      <Dialog open={!!deleteTarget} onOpenChange={() => setDeleteTarget(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>确认删除账号</DialogTitle>
            <DialogDescription>
              即将删除账号『{deleteTarget?.name}』，该操作不可恢复，但不会影响上游账号本身
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
