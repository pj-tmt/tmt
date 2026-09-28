import { expect, it } from 'vitest';
import {
  newPixelDraft,
  paintPixels,
  pixelDraftPack,
  pixelHistory,
  pixelSegment,
} from './pixel-draft.js';
import { propFrame } from './prop-contract.js';

it('keeps blank work editable but refuses to persist invisible artwork', () => {
  const blank = newPixelDraft();
  expect(pixelHistory.current(pixelHistory.create(blank))).toEqual(blank);
  expect(() => pixelDraftPack(blank)).toThrow('visible pixels');
  const painted = paintPixels(
    blank,
    [
      { x: 0, y: 0 },
      { x: 15, y: 15 },
    ],
    3
  );
  const pack = pixelDraftPack(painted);
  expect(pack).toMatchObject({
    formatVersion: 2,
    license: 'LicenseRef-Private',
    props: [{ key: 'artwork', footprint: { width: 2, height: 2 } }],
  });
  for (let rotation = 0; rotation < 4; rotation++) {
    const frame = propFrame(pack, pack.props[0]!, rotation);
    expect(frame.pixels).toEqual(painted.pixels);
    expect(frame.palette).toEqual(blank.palette);
  }
  expect(blank.pixels.every((row) => /^0+$/.test(row))).toBe(true);
});

it('commits one complete stroke, preserves palette history and erases without changing other cells', () => {
  const initial = newPixelDraft(32);
  const first = pixelHistory.create(initial);
  const cells = pixelSegment({ x: 0, y: 0 }, { x: 31, y: 31 });
  const painted = paintPixels(initial, cells, 15);
  const second = pixelHistory.commit(first, painted);
  expect(second.entries).toHaveLength(2);
  expect(painted.pixels[31]!.slice(62)).toBe('0f');
  const recolored = { ...painted, palette: [...painted.palette] };
  recolored.palette[15] = '#aabbccff';
  const third = pixelHistory.commit(second, recolored);
  recolored.palette[15] = '#ddeeffff';
  expect(pixelHistory.current(third).palette[15]).toBe('#aabbccff');
  expect(pixelHistory.current(pixelHistory.undo(third))).toEqual(painted);
  expect(pixelHistory.current(pixelHistory.redo(pixelHistory.undo(third)))).toEqual(
    pixelHistory.current(third)
  );
  const erased = paintPixels(painted, [{ x: 31, y: 31 }], 0);
  expect(erased.pixels[31]).toBe('00'.repeat(32));
  expect(erased.pixels.slice(0, 31)).toEqual(painted.pixels.slice(0, 31));
  expect(pixelHistory.current(pixelHistory.undo(second))).toEqual(initial);
});

it('bounds coordinate/palette changes and validates metadata at the existing pack boundary', () => {
  const draft = newPixelDraft();
  for (const cell of [
    { x: -1, y: 0 },
    { x: 16, y: 0 },
    { x: 1.5, y: 0 },
  ])
    expect(() => paintPixels(draft, [cell], 1)).toThrow();
  expect(() => paintPixels(draft, [], 16)).toThrow();
  expect(() => pixelSegment({ x: 0, y: 0 }, { x: 1_000_000_000, y: 0 })).toThrow();
  const painted = paintPixels(draft, [{ x: 1, y: 1 }], 1);
  expect(() => pixelDraftPack({ ...painted, label: '' })).toThrow();
  expect(() => pixelDraftPack({ ...painted, license: 'https://license.test/' })).toThrow();
  expect(() =>
    pixelHistory.commit(pixelHistory.create(draft), { ...draft, pixels: ['00'] })
  ).toThrow();
  expect(pixelSegment({ x: 3, y: 2 }, { x: 0, y: 2 })).toEqual([
    { x: 3, y: 2 },
    { x: 2, y: 2 },
    { x: 1, y: 2 },
    { x: 0, y: 2 },
  ]);
});
