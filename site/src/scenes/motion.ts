import { useSyncExternalStore } from "react";

const QUERY = "(prefers-reduced-motion: reduce)";

// One reading of the reader's motion setting for every animation on the site.
// Storage-free: the setting belongs to the browser, not to the page.
export function prefersReducedMotion(): boolean {
  try {
    return matchMedia(QUERY).matches;
  } catch {
    return false;
  }
}

function subscribe(onChange: () => void) {
  try {
    const query = matchMedia(QUERY);
    query.addEventListener("change", onChange);
    return () => query.removeEventListener("change", onChange);
  } catch {
    return () => {};
  }
}

export function useReducedMotion(): boolean {
  return useSyncExternalStore(subscribe, prefersReducedMotion, () => false);
}
