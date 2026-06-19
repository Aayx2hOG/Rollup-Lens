import type { Config } from "tailwindcss";

export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        ink: "#090b10",
        panel: "rgba(16, 20, 28, 0.78)",
        line: "rgba(255, 255, 255, 0.11)",
        mint: "#58f4b7",
        cyan: "#62d9ff",
        amber: "#f6c76b",
        rose: "#ff6f91",
      },
      boxShadow: {
        glow: "0 0 60px rgba(88, 244, 183, 0.16)",
        inset: "inset 0 1px 0 rgba(255,255,255,0.12)",
      },
      fontFamily: {
        sans: ["Inter", "ui-sans-serif", "system-ui", "sans-serif"],
        mono: ["JetBrains Mono", "ui-monospace", "SFMono-Regular", "monospace"],
      },
    },
  },
  plugins: [],
} satisfies Config;
