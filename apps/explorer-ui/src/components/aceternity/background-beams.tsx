import { motion, useReducedMotion } from "framer-motion";
import { cn } from "../../lib/utils";

// Adapted from Aceternity UI's Background Beams component for this Vite app.
export function BackgroundBeams({ className }: { className?: string }) {
  const reduceMotion = useReducedMotion();
  const paths = [
    "M-80 80 C180 40 190 260 460 210 S760 40 1120 160",
    "M-120 170 C120 110 280 330 520 250 S820 80 1160 250",
    "M-60 280 C180 190 300 400 570 310 S880 160 1180 350",
    "M40 390 C250 300 390 470 650 390 S920 270 1200 430",
  ];

  return (
    <div
      className={cn("pointer-events-none absolute inset-0 overflow-hidden", className)}
      aria-hidden="true"
    >
      <svg
        className="absolute inset-0 h-full w-full"
        viewBox="0 0 1200 520"
        fill="none"
        preserveAspectRatio="xMidYMid slice"
      >
        <defs>
          <linearGradient id="rollup-beam" x1="0" y1="0" x2="1" y2="0">
            <stop offset="0" stopColor="#58f4b7" stopOpacity="0" />
            <stop offset="0.48" stopColor="#58f4b7" stopOpacity="0.6" />
            <stop offset="0.7" stopColor="#62d9ff" stopOpacity="0.35" />
            <stop offset="1" stopColor="#62d9ff" stopOpacity="0" />
          </linearGradient>
        </defs>
        {paths.map((path, index) => (
          <g key={path}>
            <path d={path} stroke="rgba(255,255,255,0.035)" strokeWidth="1" />
            <motion.path
              d={path}
              stroke="url(#rollup-beam)"
              strokeWidth="1.25"
              strokeLinecap="round"
              initial={{ pathLength: 0.08, pathOffset: 0 }}
              animate={reduceMotion ? { pathLength: 0.2 } : { pathLength: [0.06, 0.24, 0.06], pathOffset: [0, 0.72, 1] }}
              transition={reduceMotion ? undefined : {
                duration: 7 + index * 1.2,
                delay: index * 0.8,
                repeat: Infinity,
                ease: "linear",
              }}
            />
          </g>
        ))}
      </svg>
      <div className="absolute inset-0 bg-gradient-to-r from-ink/10 via-transparent to-ink/40" />
    </div>
  );
}
