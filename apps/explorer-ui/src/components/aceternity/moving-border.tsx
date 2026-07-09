import { motion, useReducedMotion } from "framer-motion";
import type { ButtonHTMLAttributes, ReactNode } from "react";
import { cn } from "../../lib/utils";

// Adapted from Aceternity UI's Moving Border component.
export function MovingBorderButton({
  children,
  className,
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & { children: ReactNode }) {
  const reduceMotion = useReducedMotion();

  return (
    <button
      className={cn(
        "group relative inline-flex h-9 shrink-0 overflow-hidden rounded-md p-px focus:outline-none focus-visible:ring-2 focus-visible:ring-mint/50 disabled:cursor-not-allowed disabled:opacity-45",
        className,
      )}
      {...props}
    >
      <motion.span
        className="absolute -inset-[150%] bg-[conic-gradient(from_90deg_at_50%_50%,transparent_0deg,transparent_250deg,#58f4b7_300deg,transparent_360deg)]"
        animate={reduceMotion ? undefined : { rotate: 360 }}
        transition={reduceMotion ? undefined : { duration: 4, repeat: Infinity, ease: "linear" }}
        aria-hidden="true"
      />
      <span className="relative inline-flex h-full w-full items-center justify-center gap-2 rounded-[calc(0.375rem-1px)] bg-[#10141a] px-3 text-sm font-medium text-white/76 transition group-hover:bg-[#131a20] group-hover:text-mint">
        {children}
      </span>
    </button>
  );
}
