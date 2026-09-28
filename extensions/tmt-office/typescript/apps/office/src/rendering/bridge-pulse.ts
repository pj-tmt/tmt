export interface BridgeLight {
  alpha: number;
  scale: { set(value: number): void };
}

/** One bounded clock for visible bridge lights; no per-light ticker or blur. */
export function createBridgePulse(invalidate: () => void) {
  const motion = window.matchMedia('(prefers-reduced-motion: reduce)');
  let lights: readonly BridgeLight[] = [];
  let timer: number | undefined;
  let disposed = false;
  function stop() {
    if (timer !== undefined) window.clearTimeout(timer);
    timer = undefined;
  }
  function tick() {
    timer = undefined;
    if (disposed || document.hidden || motion.matches || !lights.length) return;
    const wave = Math.sin((performance.now() / 5000) * Math.PI * 2);
    for (const light of lights) {
      light.scale.set(1 + wave * 0.045);
      light.alpha = 0.92 + wave * 0.08;
    }
    invalidate();
    timer = window.setTimeout(tick, 50);
  }
  function refresh() {
    stop();
    for (const light of lights) {
      light.scale.set(1);
      light.alpha = 1;
    }
    if (!document.hidden && !disposed) invalidate();
    if (!disposed && !document.hidden && !motion.matches && lights.length)
      timer = window.setTimeout(tick, 50);
  }
  document.addEventListener('visibilitychange', refresh);
  motion.addEventListener('change', refresh);
  return {
    setLights(next: readonly BridgeLight[]) {
      if (disposed) return;
      lights = next;
      refresh();
    },
    dispose() {
      disposed = true;
      stop();
      lights = [];
      document.removeEventListener('visibilitychange', refresh);
      motion.removeEventListener('change', refresh);
    },
  };
}
