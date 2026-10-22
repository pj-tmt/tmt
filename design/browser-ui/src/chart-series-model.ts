import {
  areaPath,
  compactCount,
  type Layer,
  linePath,
  niceMax,
  stackLayers,
  ticks,
} from './chart-math.js';

export interface SeriesChartOptions {
  /** The chart's name: the figure caption, the plot's label and the data table's caption. */
  title: string;
  /**
   * `stack`: layers stacked in tokens; `share`: stacked to 100 % of each column; `bars`: one
   * series as columns; `line`: each series as a line (a missing observation breaks the line).
   */
  kind: 'stack' | 'share' | 'bars' | 'line';
  /** One label per x position, oldest first (the first, middle and last are drawn). */
  x: readonly string[];
  series: readonly Layer[];
  format?: (value: number) => string;
  /** A vertical mark at an x index: a window reset, now. */
  guides?: readonly { at: number; label: string }[];
  /** A dashed line over the same x positions: an even pace, a moving average. */
  reference?: { label: string; values: readonly (number | null)[] };
  /** A fixed top of the y axis (100 for a percent line); default is a readable step above data. */
  max?: number;
  /** Plot height in px. */
  height?: number;
  /** Shown instead of the plot when no series has an observation. */
  empty?: string;
}

/** Hue steps a chart uses, in order; the neutral step after them is the folded `other` layer. */
export const PALETTE = 7;
const OTHER_STYLE = PALETTE;
const styleOf = (layer: { key: string }, i: number) =>
  layer.key === 'other' ? OTHER_STYLE : i % PALETTE;
const MARGIN = { left: 46, right: 10, top: 12, bottom: 22 };

export interface SeriesMark {
  kind: 'area' | 'bar' | 'line';
  /** The palette step (`tmt-ui-chart-s<style>`). */
  style: number;
  d?: string;
  rect?: { x: number; y: number; width: number; height: number };
}

export interface SeriesModel {
  width: number;
  height: number;
  grid: { y: number; x1: number; x2: number; label: string }[];
  xLabels: { x: number; text: string; anchor: 'start' | 'middle' | 'end' }[];
  marks: SeriesMark[];
  reference?: string;
  guides: { x: number; top: number; bottom: number; label: string; anchor: 'start' | 'end' }[];
  /** The x position of every column, for the cursor. */
  xs: number[];
  top: number;
  bottom: number;
  /** Distance between columns, and where the first one sits, to map a pointer to a column. */
  step: number;
  left: number;
  /** One value text per series at a column (`—` for a missing observation). */
  values: (at: number) => string[];
  readout: (at: number) => string;
  legend: { label: string; style: number }[];
  /** What the hover tip shows at a column: the x label and one row per series. */
  tip: (at: number) => { title: string; rows: { label: string; value: string; style: number }[] };
  table: { columns: string[]; rows: string[][] };
}

/** Everything a series chart paints, as plain data: the painter only maps it to elements. */
export function seriesModel(options: SeriesChartOptions, width: number): SeriesModel {
  const columns = options.x.length;
  const share = options.kind === 'share';
  const format = share ? (v: number) => `${Math.round(v)}%` : (options.format ?? compactCount);
  const stack =
    options.kind === 'stack' || share ? stackLayers(options.series, { share }) : undefined;
  const dataMax = stack
    ? stack.max
    : Math.max(
        0,
        ...options.series.flatMap((layer) => layer.values.map((v) => v ?? 0)),
        ...(options.reference?.values.map((v) => v ?? 0) ?? []),
      );
  const max = options.max ?? (share ? 100 : niceMax(dataMax));
  const height = options.height ?? 200;
  const inner = {
    left: MARGIN.left,
    right: Math.max(MARGIN.left + 1, width - MARGIN.right),
    top: MARGIN.top,
    bottom: height - MARGIN.bottom,
  };
  const step = columns > 1 ? (inner.right - inner.left) / (columns - 1) : 0;
  const xAt = (i: number) => (columns > 1 ? inner.left + i * step : (inner.left + inner.right) / 2);
  const yAt = (v: number) => inner.bottom - (Math.min(v, max) / max) * (inner.bottom - inner.top);
  const xs = options.x.map((_, i) => xAt(i));
  const marks: SeriesMark[] = [];
  if (stack) {
    stack.layers.forEach((layer, i) =>
      marks.push({
        kind: 'area',
        style: styleOf(layer, i),
        d: areaPath(xs, layer.upper.map(yAt), layer.lower.map(yAt)),
      }),
    );
  } else if (options.kind === 'bars') {
    const bar = Math.max(1, Math.min(24, (inner.right - inner.left) / columns - 2));
    options.series[0]!.values.forEach((v, i) => {
      if (v == null) return;
      const y = yAt(v);
      marks.push({
        kind: 'bar',
        style: 0,
        rect: { x: xAt(i) - bar / 2, y, width: bar, height: Math.max(0, inner.bottom - y) },
      });
    });
  } else {
    options.series.forEach((layer, i) => {
      let run: [number, number][] = [];
      const flush = () => {
        if (run.length) marks.push({ kind: 'line', style: styleOf(layer, i), d: linePath(run) });
        run = [];
      };
      layer.values.forEach((v, at) => {
        if (v == null) flush();
        else run.push([xAt(at), yAt(v)]);
      });
      flush();
    });
  }
  const run: [number, number][] = [];
  options.reference?.values.forEach((v, at) => {
    if (v != null) run.push([xAt(at), yAt(v)]);
  });
  const totals = options.x.map((_, at) =>
    options.series.reduce((sum, layer) => sum + (layer.values[at] ?? 0), 0),
  );
  const cell = (layer: Layer, at: number) => {
    const v = layer.values[at];
    if (v == null) return '—';
    // In share mode a column reads as the part of its own total, never as the raw count.
    return share ? format(totals[at] ? (v / totals[at]!) * 100 : 0) : format(v);
  };
  const edge = (at: number): 'start' | 'middle' | 'end' =>
    columns > 1 && at === 0 ? 'start' : columns > 1 && at === columns - 1 ? 'end' : 'middle';
  return {
    width,
    height,
    grid: ticks(max, 4).map((tick) => ({
      y: yAt(tick),
      x1: inner.left,
      x2: inner.right,
      label: format(tick),
    })),
    xLabels: [...new Set([0, Math.floor((columns - 1) / 2), columns - 1])].map((at) => ({
      x: xAt(at),
      text: options.x[at]!,
      anchor: edge(at),
    })),
    marks,
    reference: run.length > 1 ? linePath(run) : undefined,
    guides: (options.guides ?? []).map((guide) => ({
      x: xAt(guide.at),
      // A label at the far edge sits to the left of its line so it is never clipped.
      anchor: xAt(guide.at) > inner.right - 60 ? ('end' as const) : ('start' as const),
      top: inner.top,
      bottom: inner.bottom,
      label: guide.label,
    })),
    xs,
    top: inner.top,
    bottom: inner.bottom,
    step,
    left: inner.left,
    values: (at) => options.series.map((layer) => cell(layer, at)),
    readout: (at) =>
      `${options.x[at]}: ${options.series.map((layer) => `${layer.label} ${cell(layer, at)}`).join(', ')}`,
    legend: options.series.map((layer, i) => ({ label: layer.label, style: styleOf(layer, i) })),
    tip: (at) => ({
      title: options.x[at] ?? '',
      rows: options.series.map((layer, i) => ({
        label: layer.label,
        value: cell(layer, at),
        style: styleOf(layer, i),
      })),
    }),
    table: {
      columns: [...options.x],
      rows: options.series.map((layer) => [
        layer.label,
        ...options.x.map((_, at) => cell(layer, at)),
      ]),
    },
  };
}
