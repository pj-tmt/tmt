import {
  useEffect,
  useState,
  useSyncExternalStore,
  type ComponentType,
  type RefObject,
} from "react";
import { useReducedMotion } from "./motion";

const WIDE = "(min-width: 768px)";
function subscribeWidth(update: () => void) {
  const query = matchMedia(WIDE);
  query.addEventListener("change", update);
  return () => query.removeEventListener("change", update);
}

// Spatial diagrams share one admission rule. Small/reduced-motion views keep
// their semantic flat content; the optional module loads only on screen.
export function useSpatialStage<Props extends object>(
  ref: RefObject<HTMLDivElement | null>,
  load: () => Promise<ComponentType<Props>>,
): ComponentType<Props> | null {
  const reduced = useReducedMotion();
  const wide = useSyncExternalStore(
    subscribeWidth,
    () => matchMedia(WIDE).matches,
    () => false,
  );
  const [Stage, setStage] = useState<ComponentType<Props> | null>(null);
  useEffect(() => {
    if (!wide || reduced || Stage) return;
    const element = ref.current;
    if (!element) return;
    let disposed = false;
    const start = () => {
      void load().then(
        (component) => {
          if (!disposed) setStage(() => component);
        },
        () => {
          /* A failed enhancement leaves the complete flat diagram available. */
        },
      );
    };
    const observer =
      "IntersectionObserver" in window
        ? new IntersectionObserver((entries) => {
            if (entries.some((entry) => entry.isIntersecting)) {
              observer?.disconnect();
              start();
            }
          })
        : null;
    if (observer) observer.observe(element);
    else start();
    return () => {
      disposed = true;
      observer?.disconnect();
    };
  }, [wide, reduced, Stage, ref, load]);
  return wide && !reduced ? Stage : null;
}
