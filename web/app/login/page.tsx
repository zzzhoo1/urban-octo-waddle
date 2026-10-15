"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { Activity, KeyRound } from "lucide-react";
import { toast } from "sonner";

import { setCookie } from "@/lib/api";
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

export default function LoginPage() {
  const router = useRouter();
  const [key, setKey] = useState("");
  const [loading, setLoading] = useState(false);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!key.trim()) {
      toast.error("请输入网关密钥");
      return;
    }
    setLoading(true);
    try {
      // Validate the key against the admin API before storing it.
      const res = await fetch("/admin/api/status", {
        headers: { Authorization: `Bearer ${key.trim()}` },
      });
      if (res.status === 401) {
        toast.error("密钥无效，请检查后重试");
        return;
      }
      if (!res.ok) {
        toast.error(`验证失败（${res.status}），请稍后重试`);
        return;
      }
      setCookie("gw_key", key.trim());
      toast.success("登录成功");
      router.push("/");
    } catch {
      toast.error("网络异常，无法连接网关");
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="min-h-screen flex items-center justify-center p-4">
      <Card className="w-full max-w-sm">
        <CardHeader className="items-center text-center">
          <div className="flex h-11 w-11 items-center justify-center rounded-full bg-[var(--primary)] text-[var(--primary-foreground)] mb-2">
            <Activity className="h-5 w-5" />
          </div>
          <CardTitle>CodeArts 网关管理面板</CardTitle>
          <CardDescription>请输入网关密钥（gw-key）登录</CardDescription>
        </CardHeader>
        <CardContent>
          <form onSubmit={submit} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="gwkey">网关密钥</Label>
              <div className="relative">
                <KeyRound className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-[var(--muted-foreground)]" />
                <Input
                  id="gwkey"
                  type="password"
                  placeholder="输入 gw-key"
                  className="pl-9"
                  value={key}
                  onChange={(e) => setKey(e.target.value)}
                  autoFocus
                />
              </div>
            </div>
            <Button type="submit" className="w-full" disabled={loading}>
              {loading ? "验证中..." : "登录"}
            </Button>
          </form>
        </CardContent>
      </Card>
    </div>
  );
}
