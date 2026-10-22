/**
 * The numbers behind the leaf's charts: stacking, scales, ticks, intensity levels and compact
 * counts. Pure and DOM-free, so a transform is tested without a renderer and a chart stays a thin
 * painter of what these return.
 */

/** One series over the shared x positions; `null` is a missing observation, never a zero. */
export interface Layer {
  key: string;
  label: string;
  values: readonly (number | null)[];
}

export interface StackedLayer {
  key: string;
  label: string;
  /** Bottom and top of the band at each x, in the unit of the layers (percent in share mode). */
  lower: readonly number[];
  upper: readonly number[];
  /** Where this layer had no observation (drawn as a gap in its band, not as zero). */
  missing: readonly boolean[];
}

export interface Stack {
  layers: readonly StackedLayer[];
  /** The largest top; the y domain is 0..max. 100 in share mode. */
  max: number;
}

/**
 * Stack `layers` bottom to top in the given order. In share mode every column sums to 100; a
 * column with no tokens at all stays empty rather than dividing by zero.
 */
export function stackLayers(layers: readonly Layer[], options: { share?: boolean } = {}): Stack {
  const count = Math.max(0, ...layers.map((layer) => layer.values.length));
  const totals = Array.from({ length: count }, (_, at) =>
    layers.reduce((sum, layer) => sum + (layer.values[at] ?? 0), 0),
  );
  const floor = Array.from({ length: count }, () => 0);
  const stacked = layers.map((layer): StackedLayer => {
    const lower = [...floor];
    const upper = lower.map((base, at) => {
      const raw = layer.values[at] ?? 0;
      const value = options.share ? (totals[at] ? (raw / totals[at]!) * 100 : 0) : raw;
      floor[at] = base + value;
      return base + value;
    });
    return {
      key: layer.key,
      label: layer.label,
      lower,
      upper,
      missing: Array.from({ length: count }, (_, at) => layer.values[at] == null),
    };
  });
  return { layers: stacked, max: options.share ? 100 : Math.max(0, ...floor) };
}

/**
 * Keep the `keep` largest layers by their sum and fold the rest into one `other` layer, so a
 * chart never needs more series colours than it has. The order of kept layers is largest first.
 */
export function topLayers(
  layers: readonly Layer[],
  keep: number,
  other = { key: 'other', label: 'Other' },
): Layer[] {
  const sum = (layer: Layer) => layer.values.reduce<number>((total, v) => total + (v ?? 0), 0);
  const ranked = [...layers].sort((a, b) => sum(b) - sum(a) || a.label.localeCompare(b.label));
  if (ranked.length <= keep) return ranked;
  const rest = ranked.slice(keep);
  const width = Math.max(...rest.map((layer) => layer.values.length));
  const values = Array.from({ length: width }, (_, at) => {
    const seen = rest.filter((layer) => layer.values[at] != null);
    return seen.length ? seen.reduce((total, layer) => total + (layer.values[at] ?? 0), 0) : null;
  });
  return [...ranked.slice(0, keep), { ...other, values }];
}

/** The smallest 1, 2, 2.5, 5 × 10^k at or above `max`; 1 for an empty domain. */
export function niceMax(max: number): number {
  if (!(max > 0) || !Number.isFinite(max)) return 1;
  const exponent = Math.floor(Math.log10(max));
  const base = 10 ** exponent;
  for (const step of [1, 2, 2.5, 5, 10]) if (step * base >= max - base * 1e-9) return step * base;
  return 10 * base;
}

/** `count + 1` evenly spaced ticks from 0 to `max`. */
export function ticks(max: number, count = 4): number[] {
  return Array.from({ length: count + 1 }, (_, at) => (max * at) / count);
}

/** 1.9B, 48.2M, 310K, 940: compact token and message counts. */
export function compactCount(value: number): string {
  const abs = Math.abs(value);
  if (abs >= 1_000_000_000) return `${(value / 1_000_000_000).toFixed(1)}B`;
  if (abs >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
  if (abs >= 1_000) return `${Math.round(value / 1_000)}K`;
  return String(Math.round(value));
}

/** How many intensity steps a matrix or strip uses above empty (levels 1..LEVELS). */
export const LEVELS = 5;

/**
 * Intensity of `value` against the largest `max`: null for a missing observation, 0 for a
 * known zero, otherwise 1..`steps` (LEVELS by default) on a square-root scale so small counts stay visible next to
 * one busy pair.
 */
export function level(
  value: number | null | undefined,
  max: number,
  steps = LEVELS,
): number | null {
  if (value == null) return null;
  if (value <= 0 || max <= 0) return 0;
  return Math.min(steps, Math.max(1, Math.ceil(Math.sqrt(value / max) * steps)));
}

/** SVG path data for a polyline through `points`. */
export function linePath(points: readonly (readonly [number, number])[]): string {
  return points.map(([x, y], at) => `${at ? 'L' : 'M'}${round(x)} ${round(y)}`).join('');
}

/** SVG path data for the band between an upper and a lower edge over the same x positions. */
export function areaPath(
  xs: readonly number[],
  upper: readonly number[],
  lower: readonly number[],
): string {
  if (!xs.length) return '';
  const top = xs.map((x, at) => `${at ? 'L' : 'M'}${round(x)} ${round(upper[at]!)}`).join('');
  const bottom = xs
    .map((x, at) => `L${round(x)} ${round(lower[at]!)}`)
    .reverse()
    .join('');
  return `${top}${bottom}Z`;
}

function round(value: number): number {
  return Math.round(value * 100) / 100;
}
