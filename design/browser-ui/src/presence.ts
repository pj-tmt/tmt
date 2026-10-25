import { useEffect, useState, type RefObject } from 'react';

/** Keeps an overlay mounted while its exit runs: `present` stays true after `open` turns false
 * until the element's own transitions and animations have finished, so the surface can leave
 * with the same motion it arrived with. With no motion (reduced motion, no animation API) it
 * releases at once. */
export function usePresence(open: boolean, node: RefObject<HTMLElement | null>): boolean {
  const [held, setHeld] = useState(open);
  if (open && !held) setHeld(true);
  useEffect(() => {
    if (open || !held) return;
    const running = node.current?.getAnimations?.() ?? [];
    if (!running.length) {
      setHeld(false);
      return;
    }
    let current = true;
    void Promise.allSettled(running.map((animation) => animation.finished)).then(() => {
      if (current) setHeld(false);
    });
    return () => {
      current = false;
    };
  }, [open, held, node]);
  return open || held;
}
