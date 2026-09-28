import { afterEach, expect, it, vi } from 'vitest';
import { createBridgePulse } from './bridge-pulse.js';

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

it('shares a 20 Hz clock and stops for hidden, reduced-motion, empty and disposed scenes', () => {
  vi.useFakeTimers();
  const hidden = vi.spyOn(document, 'hidden', 'get').mockReturnValue(false);
  const media = new EventTarget() as EventTarget & { matches: boolean };
  media.matches = false;
  vi.stubGlobal(
    'matchMedia',
    vi.fn(() => media as MediaQueryList)
  );
  const render = vi.fn();
  const pulse = createBridgePulse(render);
  const light = { alpha: 1, scale: { set: vi.fn() } };
  pulse.setLights([light]);
  render.mockClear();
  vi.advanceTimersByTime(1000);
  expect(render).toHaveBeenCalledTimes(20);
  expect(light.alpha).toBeGreaterThanOrEqual(0.84);
  expect(light.alpha).toBeLessThanOrEqual(1);
  for (const [scale] of light.scale.set.mock.calls) {
    expect(scale).toBeGreaterThanOrEqual(0.955);
    expect(scale).toBeLessThanOrEqual(1.045);
  }
  hidden.mockReturnValue(true);
  document.dispatchEvent(new Event('visibilitychange'));
  expect(vi.getTimerCount()).toBe(0);
  hidden.mockReturnValue(false);
  document.dispatchEvent(new Event('visibilitychange'));
  expect(vi.getTimerCount()).toBe(1);
  media.matches = true;
  media.dispatchEvent(new Event('change'));
  expect(vi.getTimerCount()).toBe(0);
  expect(light.alpha).toBe(1);
  media.matches = false;
  media.dispatchEvent(new Event('change'));
  pulse.setLights([]);
  expect(vi.getTimerCount()).toBe(0);
  pulse.setLights([light]);
  pulse.dispose();
  pulse.dispose();
  expect(vi.getTimerCount()).toBe(0);
  pulse.setLights([light]);
  media.dispatchEvent(new Event('change'));
  expect(vi.getTimerCount()).toBe(0);
});
