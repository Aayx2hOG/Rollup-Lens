import type { HTMLAttributes, ReactNode } from "react";
import { cn } from "../lib/utils";

export function Shell({ children }: { children: ReactNode }) {
  return (
    <main className="relative min-h-screen overflow-x-clip bg-ink">
      <div className="grid-mask pointer-events-none absolute inset-0" />
      <div className="beam-field pointer-events-none absolute inset-0" />
      <div className="relative mx-auto flex w-full max-w-[82rem] flex-col gap-5 px-4 py-4 sm:px-6 lg:px-8">
        {children}
      </div>
    </main>
  );
}

export function Panel({ className, children, ...props }: HTMLAttributes<HTMLDivElement>) {
  return (
    <section
      className={cn(
        "relative overflow-hidden rounded-2xl bg-white/[0.02] shadow-[0_20px_60px_rgba(0,0,0,0.18)]",
        className,
      )}
      {...props}
    >
      {children}
    </section>
  );
}

export function Stat({
  label,
  value,
  detail,
  accent = "mint",
}: {
  label: string;
  value: ReactNode;
  detail: ReactNode;
  accent?: "mint" | "cyan" | "amber" | "rose";
}) {
  const accents = {
    mint: "from-mint/24 text-mint",
    cyan: "from-cyan/24 text-cyan",
    amber: "from-amber/24 text-amber",
    rose: "from-rose/24 text-rose",
  };

  return (
    <Panel className="relative min-h-24 overflow-hidden p-4">
      <div className={cn("absolute inset-x-0 top-0 h-px bg-gradient-to-r to-transparent", accents[accent])} />
      <div className="flex items-center justify-between gap-3">
        <div className="text-xs font-medium uppercase tracking-[0.18em] text-white/44">{label}</div>
        <div className={cn("h-2 w-2 rounded-full bg-current", accents[accent])} />
      </div>
      <div className="mt-3 text-2xl font-semibold leading-none text-white sm:text-3xl">{value}</div>
      <div className="mt-2 text-sm leading-5 text-white/52">{detail}</div>
    </Panel>
  );
}

export function IconButton({
  children,
  className,
  ...props
}: React.ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      className={cn(
        "inline-flex h-10 w-10 items-center justify-center rounded-md bg-white/[0.05] text-white/75 shadow-sm transition hover:bg-mint/10 hover:text-mint focus:outline-none focus:ring-2 focus:ring-mint/50",
        className,
      )}
      {...props}
    >
      {children}
    </button>
  );
}

export function Button({
  children,
  className,
  variant = "secondary",
  size = "md",
  ...props
}: React.ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: "primary" | "secondary" | "ghost" | "warning" | "danger";
  size?: "sm" | "md" | "lg";
}) {
  return (
    <button className={buttonStyles({ variant, size, className })} {...props}>
      <span className="relative z-10 inline-flex items-center gap-[inherit]">{children}</span>
    </button>
  );
}

export function buttonStyles({
  variant = "secondary",
  size = "md",
  className,
}: {
  variant?: "primary" | "secondary" | "ghost" | "warning" | "danger";
  size?: "sm" | "md" | "lg";
  className?: string;
} = {}) {
  const variants = {
    primary: "bg-mint text-ink shadow-[0_8px_28px_rgba(88,244,183,0.14)] hover:bg-[#70f7c3] hover:shadow-[0_10px_34px_rgba(88,244,183,0.22)]",
    secondary: "bg-white/[0.045] text-white/72 shadow-sm hover:bg-white/[0.08] hover:text-white",
    ghost: "bg-transparent text-white/52 hover:bg-white/[0.06] hover:text-white",
    warning: "bg-amber text-ink shadow-[0_8px_28px_rgba(246,199,107,0.12)] hover:bg-[#f8d184]",
    danger: "bg-rose/10 text-rose hover:bg-rose/15",
  };
  const sizes = {
    sm: "h-8 gap-1.5 rounded-md px-2.5 text-xs",
    md: "h-10 gap-2 rounded-md px-3.5 text-sm",
    lg: "h-11 gap-2 rounded-lg px-5 text-sm",
  };

  return cn(
    "relative inline-flex shrink-0 items-center justify-center overflow-hidden font-medium transition duration-200",
    "after:pointer-events-none after:absolute after:inset-0 after:translate-x-[-130%] after:skew-x-[-18deg] after:bg-gradient-to-r after:from-transparent after:via-white/16 after:to-transparent after:transition-transform after:duration-500 hover:after:translate-x-[130%]",
    "active:translate-y-px focus:outline-none focus-visible:ring-2 focus-visible:ring-mint/45 focus-visible:ring-offset-2 focus-visible:ring-offset-ink",
    "disabled:cursor-not-allowed disabled:opacity-40 disabled:shadow-none disabled:after:hidden",
    variants[variant],
    sizes[size],
    className,
  );
}

export function Badge({
  children,
  tone = "neutral",
}: {
  children: ReactNode;
  tone?: "neutral" | "good" | "warn" | "bad";
}) {
  const tones = {
    neutral: "bg-white/[0.06] text-white/66",
    good: "bg-mint/10 text-mint",
    warn: "bg-amber/10 text-amber",
    bad: "bg-rose/10 text-rose",
  };

  return (
    <span className={cn("inline-flex items-center rounded-full px-2.5 py-1 text-xs font-medium", tones[tone])}>
      {children}
    </span>
  );
}

export function Skeleton({ className }: { className?: string }) {
  return <div className={cn("animate-pulse rounded-md bg-white/[0.08]", className)} />;
}
