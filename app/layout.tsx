import "./globals.css";
import Sidebar from "@/components/Sidebar";
import { Toaster } from "sonner";

export const metadata = {
  title: "AI 网关管理面板",
  description: "管理 AI 账号池、密钥、模型路由与调用日志",
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="zh-CN">
      <body>
        <div className="min-h-screen flex bg-background text-foreground">
          <Sidebar />

          <main className="flex-1">
            {children}
          </main>

          <Toaster richColors position="top-right" />
        </div>
      </body>
    </html>
  );
}
