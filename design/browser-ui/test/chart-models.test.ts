import { expect, it } from 'vite-plus/test';
import { readFileSync } from 'node:fs';
import { matrixModel } from '../src/chart-matrix-model';
import { seriesModel } from '../src/chart-series-model';
import { stripsModel } from '../src/chart-strips-model';

const css = readFileSync(new URL('../generated/static.css', import.meta.url), 'utf8');
const tokens = JSON.parse(
  readFileSync(new URL('../../tokens/tokens.json', import.meta.url), 'utf8'),
);

const x = ['00:00', '01:00', '02:00', '03:00', '04:00'];
const series = [
  { key: 'a', label: 'seat-a', values: [10, 20, null, 40, 50] },
  { key: 'b', label: 'seat-b', values: [5, 5, 5, 5, 5] },
];

it('maps a stacked chart to areas, a grid and column labels inside its plot', () => {
  const model = seriesModel({ title: 't', kind: 'stack', x, series }, 640);
  expect(model.marks.map((mark) => [mark.kind, mark.style])).toEqual([
    ['area', 0],
    ['area', 1],
  ]);
  expect(model.grid.map((line) => line.label)).toEqual(['0', '25', '50', '75', '100']);
  expect(model.xLabels.map((label) => [label.text, label.anchor])).toEqual([
    ['00:00', 'start'],
    ['02:00', 'middle'],
    ['04:00', 'end'],
  ]);
  expect(model.xs[0]).toBe(model.left);
  expect(model.xs.at(-1)).toBeLessThanOrEqual(640);
  expect(model.readout(2)).toBe('02:00: seat-a —, seat-b 5');
  expect(model.table.rows[0]).toEqual(['seat-a', '10', '20', '—', '40', '50']);
});

it('the folded other layer takes the neutral step and the tip names each series at a column', () => {
  const model = seriesModel(
    {
      title: 't',
      kind: 'stack',
      x,
      series: [series[0]!, { key: 'other', label: 'Other', values: [1, 1, 1, 1, 1] }],
    },
    640,
  );
  // Hues are tokens in order; the folded layer is the step after them, never a hue.
  expect(model.marks.map((mark) => mark.style)).toEqual([0, 7]);
  expect(model.legend.map((entry) => entry.style)).toEqual([0, 7]);
  expect(model.tip(2)).toEqual({
    title: '02:00',
    rows: [
      { label: 'seat-a', value: '—', style: 0 },
      { label: 'Other', value: '1', style: 7 },
    ],
  });
  for (let n = 1; n <= 7; n += 1) {
    const role = tokens.browser.color[`series-${n}`];
    expect(role.light).toMatch(/^#[0-9A-F]{6}$/);
    expect(css).toContain(`--tmt-ui-color-series-${n}: ${role.light.toLowerCase()}`);
  }
});

it('a single-series chart can keep a provider hue through the style option', () => {
  const line = { title: 't', kind: 'line' as const, x, series: [series[1]!] };
  expect(seriesModel({ ...line, style: 2 }, 640).marks.map((mark) => mark.style)).toEqual([2]);
  expect(seriesModel({ ...line, style: 2 }, 640).legend.map((entry) => entry.style)).toEqual([2]);
  expect(seriesModel(line, 640).marks.map((mark) => mark.style)).toEqual([0]);
});

it('share mode labels the axis in percent and keeps every band below the top', () => {
  const model = seriesModel({ title: 't', kind: 'share', x, series }, 480);
  expect(model.grid.map((line) => line.label)).toEqual(['0%', '25%', '50%', '75%', '100%']);
  // Column 0 totals 15: a reads 67 % and b 33 %, never the raw 10 and 5.
  expect(model.values(0)).toEqual(['67%', '33%']);
  expect(model.values(2)).toEqual(['—', '100%']);
  expect(model.table.rows[1]).toEqual(['seat-b', '33%', '20%', '100%', '11%', '9%']);
  for (const mark of model.marks) expect(mark.d).toMatch(/^M/);
});

it('lines break at a missing observation and a reference stays a separate dashed path', () => {
  const model = seriesModel(
    {
      title: 't',
      kind: 'line',
      x,
      series: [series[0]!],
      reference: { label: 'pace', values: [60, 50, 40, 30, 20] },
      guides: [
        { at: 1, label: 'mid' },
        { at: 4, label: 'reset' },
      ],
      max: 100,
    },
    640,
  );
  expect(model.marks.filter((mark) => mark.kind === 'line')).toHaveLength(2);
  expect(model.reference).toMatch(/^M/);
  expect(model.guides.map((guide) => [guide.label, guide.anchor])).toEqual([
    ['mid', 'start'],
    ['reset', 'end'],
  ]);
  expect(model.grid.at(-1)!.label).toBe('100');
});

it('bars skip a missing column and never draw below the axis', () => {
  const model = seriesModel({ title: 't', kind: 'bars', x, series: [series[0]!] }, 640);
  expect(model.marks).toHaveLength(4);
  for (const mark of model.marks) {
    expect(mark.rect!.height).toBeGreaterThanOrEqual(0);
    expect(mark.rect!.y + mark.rect!.height).toBeCloseTo(model.bottom, 5);
  }
});

it('a single column and a very narrow width still give finite geometry', () => {
  const model = seriesModel({ title: 't', kind: 'line', x: ['now'], series: [series[1]!] }, 20);
  for (const value of [model.left, model.xs[0]!, model.bottom])
    expect(Number.isFinite(value)).toBe(true);
  expect(model.step).toBe(0);
});

it('the matrix model keeps self blank, a known none distinct from unavailable, and sums', () => {
  const people = ['a', 'b', 'c'].map((id) => ({ id, label: `seat-${id}` }));
  const sent: Record<string, number | null> = { 'a>b': 90, 'a>c': 0, 'b>a': 1, 'b>c': null };
  const model = matrixModel({
    title: 't',
    participants: people,
    value: (from, to) => sent[`${from}>${to}`],
    sum: true,
  });
  expect(model.columns).toEqual(['seat-a', 'seat-b', 'seat-c', 'sum']);
  expect(model.rows[0]!.cells.map((cell) => cell.text)).toEqual(['', '90', '·']);
  expect(model.rows[0]!.cells[0]!.self).toBe(true);
  expect(model.rows[1]!.cells.map((cell) => cell.text)).toEqual(['1', '', '—']);
  expect(model.rows.map((row) => row.sum)).toEqual(['90', '1', '—']);
  expect(model.rows[0]!.cells[1]!.level).toBe(5);
  expect(model.rows[0]!.cells[2]!.level).toBe(0);
  expect(model.rows[1]!.cells[0]!.label).toBe('seat-b to seat-a: 1');
});

it('strips scale each row to its busiest slot, in four steps, and leave unknown slots hollow', () => {
  const model = stripsModel({
    title: 't',
    slotCount: 24,
    slotRange: (at) => `range ${at}`,
    ends: ['old', 'new'],
    rows: [{ id: 'a', label: 'seat-a', slots: [0, 4, null, 16, 1_500_000] }],
  });
  const slots = model.rows[0]!.slots;
  expect(slots).toHaveLength(24);
  // Against the busiest slot (1.5M) 4 and 16 are faint; null stays null, a known zero is step 0.
  expect(slots.slice(0, 5).map((slot) => slot.level)).toEqual([0, 1, null, 1, 4]);
  expect(Math.max(...slots.map((slot) => slot.level ?? 0))).toBe(4);
  expect(slots[2]!.tip).toEqual({ seat: 'seat-a', range: 'range 2', value: 'not observed' });
  expect(slots[4]!.tip.value).toBe('1.5M tokens');
  expect(slots[4]!.label).toBe('seat-a, range 4: 1.5M tokens');
  expect(model.rows[0]!.summary).toBe('seat-a: active in 3 of 4 observed slots');
});

/** WCAG contrast of the matrix ramp text on its fill, both themes (sRGB color-mix). */
const hex = (value: string) => [1, 3, 5].map((i) => parseInt(value.slice(i, i + 2), 16));
const lum = ([r, g, b]: number[]) => {
  const lin = (c: number) => ((c /= 255) <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
  return 0.2126 * lin(r!) + 0.7152 * lin(g!) + 0.0722 * lin(b!);
};
const ratio = (a: number[], b: number[]) => {
  const [hi, lo] = [lum(a), lum(b)].sort((p, q) => q - p);
  return (hi! + 0.05) / (lo! + 0.05);
};
const mixOf = (text: number[], sheet: number[], percent: number) =>
  text.map((channel, at) => (channel * percent + sheet[at]! * (100 - percent)) / 100);

it('keeps matrix text readable on every intensity step in light and dark', () => {
  for (const theme of ['light', 'dark'] as const) {
    const text = hex(tokens.browser.color.text[theme]);
    const sheet = hex(tokens.browser.surface.sheet[theme]);
    for (const [step, percent] of [
      [1, 10],
      [2, 20],
      [3, 30],
      [4, 72],
      [5, 88],
    ] as const) {
      const rule = new RegExp(
        `\\.tmt-ui-chart-level-${step} \\{\\s*background: color-mix\\(in srgb, var\\(--tmt-ui-color-text\\) ${percent}%[^;]*;\\s*color: var\\(--tmt-ui-(color-text|surface-sheet)\\)`,
      ).exec(css);
      expect(rule, `level ${step} rule`).not.toBeNull();
      const fill = mixOf(text, sheet, percent);
      const ink = rule![1] === 'color-text' ? text : sheet;
      expect(ratio(ink, fill), `${theme} level ${step}`).toBeGreaterThanOrEqual(4.5);
    }
  }
});

it('every chart class a painter writes exists in the generated stylesheet', () => {
  const written = new Set<string>();
  for (const file of ['chart-dom', 'chart-series', 'chart-matrix', 'chart-strips']) {
    const source = readFileSync(new URL(`../src/${file}.ts`, import.meta.url), 'utf8');
    for (const match of source.matchAll(/tmt-ui-chart-[a-z0-9-]+/g)) written.add(match[0]);
  }
  const dynamic = /-(?:s|level-|slot-|strip-)$/;
  expect(written.size).toBeGreaterThan(20);
  for (const name of written) {
    if (dynamic.test(name)) continue;
    expect(css, name).toContain(`.${name}`);
  }
});
