import { useLayoutEffect, useRef, useState, type ReactNode } from "react";

// Keeps a terminal screen's columns intact on a narrow display: below `width`
// the screen is zoomed down to fit, never below `minimum`, and scrolls sideways
// past that. At `width` or wider it fills its container.
export function FitWidth({
  width,
  minimum = 0.72,
  children,
}: {
  width: number;
  minimum?: number;
  children: ReactNode;
}) {
  const outer = useRef<HTMLDivElement>(null);
  const [zoom, setZoom] = useState(1);
  useLayoutEffect(() => {
    const element = outer.current;
    if (!element) return;
    const fit = () => setZoom(Math.max(minimum, Math.min(1, element.clientWidth / width)));
    fit();
    const observer = new ResizeObserver(fit);
    observer.observe(element);
    return () => observer.disconnect();
  }, [width, minimum]);
  return (
    <div ref={outer} className="term-scroll w-full">
      <div style={zoom < 1 ? { width, zoom } : undefined}>{children}</div>
    </div>
  );
}
