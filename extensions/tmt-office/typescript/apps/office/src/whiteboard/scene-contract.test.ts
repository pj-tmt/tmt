import { describe, expect, it } from 'vitest';
import example from '../../../../../../../contracts/office/whiteboard-scene-v1.json';
import vectors from '../../../../../../../contracts/office/whiteboard-vectors.json';
import {
  decodeWhiteboardScene,
  parseWhiteboardScene,
  WHITEBOARD_LIMITS,
} from './scene-contract.js';

const note = example.elements[0]!;
const shape = example.elements[1]!;
const path = example.elements[3]!;
const scene = (elements: unknown[]) => ({ ...example, elements });
const id = (index: number) => `${index.toString(16).padStart(8, '0')}-1111-4111-8111-111111111111`;

describe('whiteboard scene admission', () => {
  it.each(vectors)('matches the shared native vector: $name', ({ value, valid }) => {
    if (valid) expect(decodeWhiteboardScene(value)).toEqual(value);
    else expect(() => decodeWhiteboardScene(value)).toThrow();
  });

  it('round trips every supported element without changing paint order, IDs or text', () => {
    expect(parseWhiteboardScene(JSON.stringify(example))).toEqual(example);
    expect(decodeWhiteboardScene(scene([])).elements).toEqual([]);
  });

  it('admits equivalent whole-number notation but never rounds fractional input', () => {
    const source = JSON.stringify(example);
    for (const [before, after] of [
      ['"formatVersion":1', '"formatVersion":1.0'],
      ['"width":1600', '"width":1.6e3'],
      ['[650,450]', '[650.0,4.5e2]'],
    ]) {
      expect(source).toContain(before);
      expect(parseWhiteboardScene(source.replace(before!, after!))).toEqual(example);
    }
    expect(() => parseWhiteboardScene(source.replace('"width":1600', '"width":1600.1'))).toThrow();
  });

  it('owns its input instead of retaining mutable element or point references', () => {
    const input = structuredClone(example);
    const result = decodeWhiteboardScene(input);
    input.elements[0]!.text = 'Changed outside the document';
    input.elements[3]!.points![0]![0] = 0;
    expect(result).toEqual(example);
  });

  it.each([
    ['version', { ...example, formatVersion: 2 }],
    ['surface size', { ...example, width: 1601 }],
    ['remote background', { ...example, background: 'https://example.invalid/image.png' }],
    ['unknown field', { ...example, script: 'run()' }],
    ['null elements', { ...example, elements: null }],
    ['duplicate IDs', scene([note, note])],
    ['unknown kind', scene([{ ...note, kind: 'html' }])],
    ['unknown element field', scene([{ ...note, onclick: 'run()' }])],
    ['noncanonical ID', scene([{ ...note, id: 'Alice' }])],
    ['fractional position', scene([{ ...note, x: 0.5 }])],
    ['negative position', scene([{ ...note, x: -1 }])],
    ['zero width', scene([{ ...shape, width: 0 }])],
    ['overflowing box', scene([{ ...shape, x: 1599, width: 2 }])],
    ['invalid fill', scene([{ ...shape, fill: 'url(#anything)' }])],
    ['oversized stroke', scene([{ ...path, strokeWidth: 17 }])],
    ['one-point path', scene([{ ...path, points: [[10, 10]] }])],
    [
      'overflowing point',
      scene([
        {
          ...path,
          points: [
            [0, 0],
            [1601, 1000],
          ],
        },
      ]),
    ],
    [
      'extra point coordinate',
      scene([
        {
          ...path,
          points: [
            [0, 0, 1],
            [10, 10],
          ],
        },
      ]),
    ],
    ['arrow with a third point', scene([{ ...path, kind: 'arrow' }])],
    ['null text', scene([{ ...note, text: null }])],
    ['control text', scene([{ ...note, text: 'before\u0000after' }])],
    ['unpaired surrogate', scene([{ ...note, text: '\ud800' }])],
    ['font size', scene([{ ...note, fontSize: 73 }])],
  ])('rejects %s without another unrelated fault', (_, value) => {
    expect(() => decodeWhiteboardScene(value)).toThrow();
  });

  it('admits surface-edge geometry and inert multiline markup literally', () => {
    const elements = [
      { ...shape, x: 1599, y: 999, width: 1, height: 1 },
      {
        ...path,
        points: [
          [0, 0],
          [1600, 1000],
        ],
      },
      { ...note, text: '<script>not executable</script>\n\t🙂' },
    ];
    expect(decodeWhiteboardScene(scene(elements)).elements).toEqual(elements);
  });

  it('uses UTF-8 text budgets and accepts the exact per-element boundary', () => {
    const text = '🙂'.repeat(WHITEBOARD_LIMITS.textBytes / 4);
    expect(decodeWhiteboardScene(scene([{ ...note, text }])).elements).toHaveLength(1);
    expect(() => decodeWhiteboardScene(scene([{ ...note, text: text + 'x' }]))).toThrow();
  });

  it('limits aggregate work, not only each individual element', () => {
    const notes = Array.from({ length: 4 }, (_, index) => ({
      ...note,
      id: id(index),
      text: 'x'.repeat(WHITEBOARD_LIMITS.textBytes),
    }));
    expect(decodeWhiteboardScene(scene(notes)).elements).toHaveLength(4);
    expect(() => decodeWhiteboardScene(scene([...notes, { ...note, text: 'x' }]))).toThrow();

    const paths = Array.from({ length: 2 }, (_, index) => ({
      ...path,
      id: id(index),
      points: Array.from({ length: WHITEBOARD_LIMITS.points / 2 }, () => [0, 0]),
    }));
    expect(decodeWhiteboardScene(scene(paths)).elements).toHaveLength(2);
    expect(() => decodeWhiteboardScene(scene([...paths, path]))).toThrow();
  });

  it('bounds element count and raw document bytes, including irrelevant whitespace', () => {
    const elements = Array.from({ length: WHITEBOARD_LIMITS.elements }, (_, index) => ({
      ...shape,
      id: id(index),
    }));
    expect(decodeWhiteboardScene(scene(elements)).elements).toHaveLength(
      WHITEBOARD_LIMITS.elements
    );
    expect(() => decodeWhiteboardScene(scene([...elements, shape]))).toThrow();
    const source = JSON.stringify(scene([]));
    expect(parseWhiteboardScene(source.padEnd(WHITEBOARD_LIMITS.documentBytes))).toEqual(scene([]));
    expect(() =>
      parseWhiteboardScene(source.padEnd(WHITEBOARD_LIMITS.documentBytes + 1))
    ).toThrow();
  });
});
