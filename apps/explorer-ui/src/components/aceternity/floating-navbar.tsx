import type { ReactNode } from "react";
import { cn } from "../../lib/utils";

export function FloatingNavbar({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <header className={cn("relative z-40 md:sticky md:top-3", className)}>
      {children}
    </header>
  );
}
