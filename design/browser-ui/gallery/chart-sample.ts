/** Deterministic sample series for the chart examples: the same bytes on every load. */
import type { Layer } from '../src/chart-math';

function random(seed: number) {
  let state = seed >>> 0;
  return () => {
    state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
    return state / 2 ** 32;
  };
}

export const hours = Array.from({ length: 24 }, (_, at) => `${String(at).padStart(2, '0')}:00`);

export function seatLayers(count: number): Layer[] {
  const next = random(7);
  return Array.from({ length: count }, (_, seat) => ({
    key: `seat-${seat + 1}`,
    label: `seat-${seat + 1}`,
    values: hours.map((_, at) =>
      seat === 2 && at > 20 ? null : Math.round(next() * (2_400_000 / (seat + 1)) + 20_000),
    ),
  }));
}

export const weeklyX = Array.from(
  { length: 8 * 6 },
  (_, at) => `d${Math.floor(at / 6)} ${(at % 6) * 4}h`,
);
export const weeklyLeft: (number | null)[] = weeklyX.map((_, at) =>
  at === 20 || at === 21 ? null : Math.max(0, 100 - at * 1.55),
);
export const weeklyPace = weeklyX.map((_, at) => 100 - (at * 100) / (weeklyX.length - 1));

export const burn = hours.map((_, at) => (at % 7 === 3 ? 0 : 0.4 + ((at * 37) % 11) / 10));
export const burnAverage = burn.map((_, at) => {
  const window = burn.slice(Math.max(0, at - 5), at + 1);
  return window.reduce((sum, v) => sum + v, 0) / window.length;
});

const names = ['lead', 'worker-1', 'worker-2', 'reviewer', 'night'];
export const people = names.map((name) => ({ id: name, label: name }));
export const manyPeople = Array.from({ length: 9 }, (_, at) => ({
  id: `agent-${at}`,
  label: `agent-with-a-long-name-${at}`,
}));
export function sentBetween(from: string, to: string): number | null {
  if (from === 'night' || to === 'night') return null;
  const seed = [...(from + to)].reduce((sum, c) => sum + c.charCodeAt(0), 0);
  return seed % 5 === 0 ? 0 : (seed * 13) % 240;
}

export const slotRange = (at: number) =>
  `${String(at).padStart(2, '0')}:00–${String((at + 1) % 24).padStart(2, '0')}:00`;
export function strips(rows: number) {
  const next = random(11);
  return Array.from({ length: rows }, (_, at) => ({
    id: `seat-${at}`,
    label: at === 1 ? 'a-rather-long-seat-name-for-the-row' : `seat-${at + 1}`,
    slots: Array.from({ length: 24 }, (_, slot) =>
      at === 3 && slot < 10 ? null : next() < 0.35 ? 0 : Math.round(next() * 40),
    ),
  }));
}
