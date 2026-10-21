/** Busy shorter than this changes nothing visually. */
export const busyMarkDelayMs = 400;
/** Once shown, the busy mark stays at least this long. */
export const busyMarkHoldMs = 400;

/**
 * Decides when a busy mark is visible: only after busy has lasted the delay,
 * then for at least the hold, so short work never flashes a mark.
 */
export function createBusyMarkGate(onChange: (shown: boolean) => void) {
  let busy = false;
  let shown = false;
  let shownAt = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const clear = () => {
    if (timer !== undefined) clearTimeout(timer);
    timer = undefined;
  };
  const show = (value: boolean) => {
    shown = value;
    if (value) shownAt = Date.now();
    onChange(value);
  };
  return {
    shown: () => shown,
    set(next: boolean) {
      if (next === busy) return;
      busy = next;
      clear();
      if (busy) {
        if (!shown) timer = setTimeout(() => show(true), busyMarkDelayMs);
        return;
      }
      if (!shown) return;
      const remaining = shownAt + busyMarkHoldMs - Date.now();
      if (remaining <= 0) show(false);
      else timer = setTimeout(() => show(false), remaining);
    },
    /** Stops pending timers; a later busy start begins a fresh delay. */
    dispose() {
      clear();
      busy = false;
    },
  };
}
