import { cn } from "@/lib/utils";

/**
 * 骨架屏：基础脉冲 + 流光扫过（shimmer）双效果，
 * 让加载等待更有质感。
 */
function Skeleton({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cn("relative overflow-hidden rounded-md bg-[var(--muted)] animate-pulse", className)}
      {...props}
    >
      <div className="absolute inset-0 -translate-x-full animate-[shimmer_1.6s_infinite] bg-gradient-to-r from-transparent via-white/15 to-transparent" />
    </div>
  );
}

export { Skeleton };
