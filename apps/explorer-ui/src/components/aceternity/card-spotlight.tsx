import { useState, type HTMLAttributes, type MouseEvent } from "react";
import { cn } from "../../lib/utils";

// Adapted from Aceternity UI's Card Spotlight component.
export function CardSpotlight({
  children,
  className,
  radius = 280,
  ...props
}: HTMLAttributes<HTMLDivElement> & { radius?: number }) {
  const [position, setPosition] = useState({ x: 0, y: 0 });
  const [visible, setVisible] = useState(false);

  function trackPointer(event: MouseEvent<HTMLDivElement>) {
    const bounds = event.currentTarget.getBoundingClientRect();
    setPosition({ x: event.clientX - bounds.left, y: event.clientY - bounds.top });
  }

  return (
    <div
      className={cn("relative overflow-hidden", className)}
      onMouseMove={trackPointer}
      onMouseEnter={() => setVisible(true)}
      onMouseLeave={() => setVisible(false)}
      {...props}
    >
      <div
        className="pointer-events-none !absolute -inset-px transition-opacity duration-300"
        style={{
          opacity: visible ? 1 : 0,
          background: `radial-gradient(${radius}px circle at ${position.x}px ${position.y}px, rgba(88,244,183,0.11), transparent 72%)`,
        }}
        aria-hidden="true"
      />
      <div className="relative z-10 h-full">{children}</div>
    </div>
  );
}
