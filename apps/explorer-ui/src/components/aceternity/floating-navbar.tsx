import { useEffect, useRef, useState, type ReactNode } from "react";
import { motion, useReducedMotion } from "framer-motion";
import { cn } from "../../lib/utils";

// Adapted from Aceternity UI's Floating Navbar component.
export function FloatingNavbar({
  children,
  className,
}: {
  children: ReactNode;
  className?: string;
}) {
  const [visible, setVisible] = useState(true);
  const previousY = useRef(0);
  const reduceMotion = useReducedMotion();

  useEffect(() => {
    function onScroll() {
      const currentY = window.scrollY;
      const movingUp = currentY < previousY.current;
      setVisible(currentY < 72 || movingUp);
      previousY.current = currentY;
    }

    window.addEventListener("scroll", onScroll, { passive: true });
    return () => window.removeEventListener("scroll", onScroll);
  }, []);

  return (
    <motion.header
      initial={false}
      animate={reduceMotion ? undefined : { y: visible ? 0 : -110, opacity: visible ? 1 : 0 }}
      transition={{ duration: 0.24, ease: "easeOut" }}
      className={cn("sticky top-3 z-40", className)}
    >
      {children}
    </motion.header>
  );
}
