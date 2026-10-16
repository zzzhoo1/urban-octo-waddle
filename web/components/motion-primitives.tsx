"use client";

import { useEffect, useRef, useState } from "react";
import { motion, useInView, useSpring, useTransform } from "motion/react";

import { cn } from "@/lib/utils";

/**
 * 数字滚动动画：数值变化时像计数器一样平滑滚动到新值。
 * 用于汇总卡片，让数据更新有生命感。
 */
export function AnimatedNumber({
  value,
  format = (n: number) => String(Math.round(n)),
  className,
}: {
  value: number;
  format?: (n: number) => string;
  className?: string;
}) {
  const spring = useSpring(0, { stiffness: 80, damping: 20 });
  const display = useTransform(spring, (v) => format(v));
  const [text, setText] = useState(format(0));

  useEffect(() => {
    spring.set(value);
  }, [spring, value]);

  useEffect(() => {
    const unsub = display.on("change", (v) => setText(v));
    return unsub;
  }, [display]);

  return <span className={className}>{text}</span>;
}

/**
 * 悬浮微抬升卡片：鼠标悬停时轻轻上浮并加深柔光阴影，
 * 配合 200ms 缓动，是现代化控制台的标志性细节。
 */
export function HoverCard({
  children,
  className,
}: {
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <motion.div
      whileHover={{ y: -3, transition: { duration: 0.2, ease: "easeOut" } }}
      whileTap={{ scale: 0.99 }}
      className={cn(
        "transition-shadow duration-200 hover:shadow-lg hover:shadow-[var(--primary)]/10",
        className
      )}
    >
      {children}
    </motion.div>
  );
}

/**
 * 入场淡入容器：进入视口时淡入 + 上移，
 * 支持逐个延迟，替代手写 initial/animate 重复代码。
 */
export function FadeIn({
  children,
  delay = 0,
  className,
}: {
  children: React.ReactNode;
  delay?: number;
  className?: string;
}) {
  return (
    <motion.div
      initial={{ opacity: 0, y: 12 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ delay, duration: 0.35, ease: "easeOut" }}
      className={className}
    >
      {children}
    </motion.div>
  );
}

/**
 * 状态脉冲点：健康/活跃状态的呼吸脉冲，
 * 用于账号健康指示，一眼看出服务活着。
 */
export function PulseDot({
  color = "bg-green-500",
  className,
}: {
  color?: string;
  className?: string;
}) {
  return (
    <span className={cn("relative flex h-2.5 w-2.5", className)}>
      <span
        className={cn(
          "animate-ping absolute inline-flex h-full w-full rounded-full opacity-60",
          color
        )}
      />
      <span className={cn("relative inline-flex rounded-full h-2.5 w-2.5", color)} />
    </span>
  );
}

/**
 * 数据刷新微光：数据刷新瞬间闪过一道微光，
 * 让用户感知到数据是最新的（用于统计卡片与日志页）。
 */
export function useFlashOnChange<T>(value: T): boolean {
  const [flash, setFlash] = useState(false);
  const prev = useRef(value);
  useEffect(() => {
    if (prev.current !== value) {
      prev.current = value;
      setFlash(true);
      const t = setTimeout(() => setFlash(false), 600);
      return () => clearTimeout(t);
    }
  }, [value]);
  return flash;
}

export function FlashWrapper({
  watch,
  children,
  className,
}: {
  watch: unknown;
  children: React.ReactNode;
  className?: string;
}) {
  const flash = useFlashOnChange(watch);
  return (
    <div
      className={cn(
        "transition-colors duration-500 rounded-lg",
        flash && "bg-[var(--primary)]/5",
        className
      )}
    >
      {children}
    </div>
  );
}

/**
 * 骨架呼吸屏的渐进增强：加载时骨架从左到右的流光扫过，
 * 比普通脉冲更有质感。
 */
export function ShimmerBar({ className }: { className?: string }) {
  return (
    <div className={cn("relative overflow-hidden rounded-md bg-[var(--muted)]", className)}>
      <motion.div
        className="absolute inset-0 -translate-x-full bg-gradient-to-r from-transparent via-white/20 to-transparent"
        animate={{ x: ["-100%", "100%"] }}
        transition={{ repeat: Infinity, duration: 1.4, ease: "linear" }}
      />
      <div className="h-full w-full" />
    </div>
  );
}

/**
 * 页面切换计数徽标：导航上显示动态数字（如未读/告警数），
 * 数字变化时弹跳一次提醒注意。
 */
export function BounceBadge({
  value,
  className,
}: {
  value: number;
  className?: string;
}) {
  return (
    <motion.span
      key={value}
      initial={{ scale: 1.4 }}
      animate={{ scale: 1 }}
      transition={{ type: "spring", stiffness: 500, damping: 15 }}
      className={cn(
        "inline-flex items-center justify-center rounded-full bg-[var(--primary)] text-[var(--primary-foreground)] text-[10px] font-bold min-w-4 h-4 px-1",
        className
      )}
    >
      {value}
    </motion.span>
  );
}

export { useInView };
