import type { HTMLAttributes, ReactNode } from "react";
import { cn } from "../lib/utils";

export function Shell({ children }: { children: ReactNode }) {
  return (
    <main className="relative min-h-screen overflow-hidden bg-ink">
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
    <section className={cn("surface rounded-2xl", className)} {...props}>
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
    <Panel className="relative min-h-28 overflow-hidden p-4">
      <div className={cn("absolute inset-x-0 top-0 h-px bg-gradient-to-r to-transparent", accents[accent])} />
      <div className="flex items-center justify-between gap-3">
        <div className="text-xs font-medium uppercase tracking-[0.18em] text-white/44">{label}</div>
        <div className={cn("h-2 w-2 rounded-full bg-current", accents[accent])} />
      </div>
      <div className="mt-3 text-3xl font-semibold leading-none text-white">{value}</div>
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
        "inline-flex h-10 w-10 items-center justify-center rounded-lg border border-white/10 bg-white/[0.05] text-white/75 shadow-sm transition hover:border-mint/40 hover:bg-mint/10 hover:text-mint focus:outline-none focus:ring-2 focus:ring-mint/50",
        className,
      )}
      {...props}
    >
      {children}
    </button>
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
    neutral: "border-white/10 bg-white/[0.06] text-white/66",
    good: "border-mint/25 bg-mint/10 text-mint",
    warn: "border-amber/25 bg-amber/10 text-amber",
    bad: "border-rose/25 bg-rose/10 text-rose",
  };

  return (
    <span className={cn("inline-flex items-center rounded-full border px-2.5 py-1 text-xs font-medium", tones[tone])}>
      {children}
    </span>
  );
}

export function Skeleton({ className }: { className?: string }) {
  return <div className={cn("animate-pulse rounded-md bg-white/[0.08]", className)} />;
}
