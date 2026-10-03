import { useEffect, useRef, useState, type RefObject } from "react";
import { useReducedMotion } from "./motion";

type Options<T extends HTMLElement> = {
  intervalMs: number;
  // The complete frame shown, with no timer, under reduced motion.
  rest: number;
  // Hold the current frame, for example while the reader points at the scene.
  paused?: boolean;
  ref?: RefObject<T | null>;
};

// Frames of one scene, advancing on a timer only while the scene is on screen.
// A scene is data (one complete frame per index); this hook only picks the
// index. Choosing a frame by hand ends the automatic advance for good.
export function useFrames<T extends HTMLElement>(
  count: number,
  { intervalMs, rest, paused = false, ref: sharedRef }: Options<T>,
): {
  ref: RefObject<T | null>;
  index: number;
  choose: (index: number) => void;
} {
  const localRef = useRef<T | null>(null);
  const ref = sharedRef ?? localRef;
  const reduced = useReducedMotion();
  const [tick, setTick] = useState(0);
  const [chosen, setChosen] = useState<number | null>(null);
  const [foreground, setForeground] = useState(() => !document.hidden);
  // Without IntersectionObserver the scene counts as always on screen.
  const [visible, setVisible] = useState(() => !("IntersectionObserver" in window));

  useEffect(() => {
    const element = ref.current;
    if (!element) return;
    if (!("IntersectionObserver" in window)) return;
    const observer = new IntersectionObserver(
      (entries) => setVisible(entries.some((entry) => entry.isIntersecting)),
      { threshold: 0 },
    );
    observer.observe(element);
    return () => observer.disconnect();
  }, [ref]);

  useEffect(() => {
    const update = () => setForeground(!document.hidden);
    document.addEventListener("visibilitychange", update);
    return () => document.removeEventListener("visibilitychange", update);
  }, []);

  const running = !reduced && chosen === null && visible && foreground && !paused;
  useEffect(() => {
    if (!running) return;
    const timer = setInterval(() => setTick((value) => (value + 1) % count), intervalMs);
    return () => clearInterval(timer);
  }, [running, count, intervalMs]);

  return {
    ref,
    index: chosen ?? (reduced ? rest : tick),
    choose: setChosen,
  };
}
