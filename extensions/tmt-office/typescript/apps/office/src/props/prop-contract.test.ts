import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import builtinDocument from '../../../../../contracts/builtin-props-v1.tmtprop.json' with { type: 'json' };
import vectors from '../../../../../contracts/prop-block-vectors.json' with { type: 'json' };
import {
  BUILTIN_DIGEST,
  BUILTIN_PACK,
  BUILTIN_CATALOG,
  WORKSHOP_DIGEST,
  COMMONS_DIGEST,
  WHITEBOARD_DIGEST,
  BROADCASTER_DIGEST,
  STUDY_DIGEST,
  WALL_DIGEST,
  MODULAR_WORKSTATION_DIGEST,
  MODULAR_MOUNTED_DIGEST,
  MODULAR_LOUNGE_DIGEST,
  MODULAR_FACILITIES_DIGEST,
  MODULAR_RECEPTION_DIGEST,
  DIRECTIONAL_WORKSTATION_DIGEST,
  DIRECTIONAL_LOUNGE_DIGEST,
  DIRECTIONAL_RECEPTION_DIGEST,
  DIRECTIONAL_FACILITIES_DIGEST,
  decodePropPack,
  indexedProp,
} from './prop-contract.js';

describe('data-only prop pack contract', () => {
  it.each([
    MODULAR_MOUNTED_DIGEST,
    MODULAR_LOUNGE_DIGEST,
    MODULAR_FACILITIES_DIGEST,
    MODULAR_RECEPTION_DIGEST,
  ])('keeps %s static, nonempty and surrounded by transparent guards', (digest) => {
    const pack = BUILTIN_CATALOG.find((entry) => entry.digest === digest)!.pack;
    for (const prop of pack.props) {
      const frames = prop.frames!;
      expect(new Set(frames.map((frame) => JSON.stringify(frame))).size).toBe(1);
      expect(frames[0].some((row) => /[1-9a-f]/.test(row))).toBe(true);
      for (const frame of frames) {
        expect(frame[0]).toMatch(/^0+$/);
        expect(frame.at(-1)).toMatch(/^0+$/);
        expect(frame.every((row) => row.startsWith('00') && row.endsWith('00'))).toBe(true);
      }
    }
  });
  it('retains four authored chair views, static desk art and transparent frame guards', () => {
    const pack = BUILTIN_CATALOG.find((entry) => entry.digest === MODULAR_WORKSTATION_DIGEST)!.pack;
    for (const prop of pack.props) {
      const frames = prop.frames!;
      expect(new Set(frames.map((frame) => JSON.stringify(frame))).size).toBe(
        prop.key === 'workstation-chair' ? 4 : 1
      );
      for (const frame of frames) {
        expect(frame[0]).toMatch(/^0+$/);
        expect(frame.at(-1)).toMatch(/^0+$/);
        expect(frame.every((row) => row.startsWith('00') && row.endsWith('00'))).toBe(true);
      }
    }
  });
  it('faces the chair west and east instead of trusting the original sheet column labels', () => {
    const pack = BUILTIN_CATALOG.find((entry) => entry.digest === MODULAR_WORKSTATION_DIGEST)!.pack;
    const frames = indexedProp(pack, 'workstation-chair')!.frames!;
    function upperBackCenter(frame: string[]) {
      const occupied = frame
        .slice(0, 21)
        .flatMap((row) =>
          Array.from({ length: 64 }, (_, x) =>
            row.slice(x * 2, x * 2 + 2) !== '00' ? [x] : []
          ).flat()
        );
      return occupied.reduce((sum, x) => sum + x, 0) / occupied.length;
    }
    // Looking west, the high back is to the right of the seat; east is reversed.
    expect(upperBackCenter(frames[1])).toBeGreaterThan(32);
    expect(upperBackCenter(frames[3])).toBeLessThan(32);
  });
  it.each([
    {
      file: 'modular-reception-v2.tmtprop.json',
      digest: MODULAR_RECEPTION_DIGEST,
      index: 11,
      keys: ['reception-armchair', 'reception-table'],
    },
    {
      file: 'modular-facilities-v2.tmtprop.json',
      digest: MODULAR_FACILITIES_DIGEST,
      index: 10,
      keys: ['lobby-whiteboard', 'lobby-discussion-board', 'lobby-radio'],
    },
    {
      file: 'modular-lounge-v2.tmtprop.json',
      digest: MODULAR_LOUNGE_DIGEST,
      index: 9,
      keys: ['lounge-sofa', 'lounge-armchair', 'lounge-table', 'lounge-plant'],
    },
    {
      file: 'modular-mounted-v2.tmtprop.json',
      digest: MODULAR_MOUNTED_DIGEST,
      index: 8,
      keys: ['mounted-window', 'mounted-sconce', 'mounted-frame', 'mounted-shelf'],
    },
    {
      file: 'modular-workstation-v2.tmtprop.json',
      digest: MODULAR_WORKSTATION_DIGEST,
      index: 7,
      keys: [
        'workstation-desk',
        'workstation-chair',
        'workstation-terminal',
        'workstation-bookcase',
      ],
    },
    {
      file: 'wall-props-v2.tmtprop.json',
      digest: WALL_DIGEST,
      index: 6,
      keys: ['observatory-window', 'brass-wall-lamp', 'orbit-poster', 'crew-sign', 'link-plaque'],
    },
    {
      file: 'workshop-furniture-v2.tmtprop.json',
      digest: WORKSHOP_DIGEST,
      index: 1,
      keys: ['oak-desk', 'green-chair', 'leafy-plant', 'woven-rug', 'lounge-sofa', 'coffee-table'],
    },
    {
      file: 'commons-props-v2.tmtprop.json',
      digest: COMMONS_DIGEST,
      index: 2,
      keys: ['noticeboard'],
    },
    {
      file: 'whiteboard-props-v2.tmtprop.json',
      digest: WHITEBOARD_DIGEST,
      index: 3,
      keys: ['whiteboard'],
    },
    {
      file: 'broadcaster-props-v2.tmtprop.json',
      digest: BROADCASTER_DIGEST,
      index: 4,
      keys: ['broadcast-station'],
    },
    {
      file: 'study-furniture-v2.tmtprop.json',
      digest: STUDY_DIGEST,
      index: 5,
      keys: ['oak-bookcase', 'reading-lamp', 'desktop-terminal'],
    },
    {
      file: 'directional-workstation-v2.tmtprop.json',
      digest: DIRECTIONAL_WORKSTATION_DIGEST,
      index: 12,
      keys: ['workstation-desk', 'workstation-terminal', 'workstation-bookcase'],
    },
    {
      file: 'directional-lounge-v2.tmtprop.json',
      digest: DIRECTIONAL_LOUNGE_DIGEST,
      index: 13,
      keys: ['lounge-sofa', 'lounge-armchair', 'lounge-table', 'lounge-plant'],
    },
    {
      file: 'directional-reception-v2.tmtprop.json',
      digest: DIRECTIONAL_RECEPTION_DIGEST,
      index: 14,
      keys: ['reception-armchair', 'reception-table'],
    },
    {
      file: 'directional-facilities-v2.tmtprop.json',
      digest: DIRECTIONAL_FACILITIES_DIGEST,
      index: 15,
      keys: ['lobby-whiteboard', 'lobby-discussion-board', 'lobby-radio'],
    },
  ])('admits $file with its frozen v2 identity without replacing existing art', (fixture) => {
    const bytes = readFileSync(path.resolve(process.cwd(), '../../../contracts', fixture.file));
    const length = Buffer.alloc(8);
    length.writeBigUInt64BE(BigInt(bytes.length));
    const digest = createHash('sha256')
      .update(Buffer.from('TMT-OFFICE-PROP-PACK-V2\0'))
      .update(length)
      .update(bytes)
      .digest('hex');
    expect(`sha256:${digest}`).toBe(fixture.digest);
    expect(BUILTIN_CATALOG.map((entry) => entry.digest)).toEqual([
      BUILTIN_DIGEST,
      WORKSHOP_DIGEST,
      COMMONS_DIGEST,
      WHITEBOARD_DIGEST,
      BROADCASTER_DIGEST,
      STUDY_DIGEST,
      WALL_DIGEST,
      MODULAR_WORKSTATION_DIGEST,
      MODULAR_MOUNTED_DIGEST,
      MODULAR_LOUNGE_DIGEST,
      MODULAR_FACILITIES_DIGEST,
      MODULAR_RECEPTION_DIGEST,
      DIRECTIONAL_WORKSTATION_DIGEST,
      DIRECTIONAL_LOUNGE_DIGEST,
      DIRECTIONAL_RECEPTION_DIGEST,
      DIRECTIONAL_FACILITIES_DIGEST,
    ]);
    const pack = BUILTIN_CATALOG[fixture.index]!.pack;
    expect(pack).toEqual(decodePropPack(JSON.parse(bytes.toString('utf8'))));
    expect(pack.props.map((prop) => prop.key)).toEqual(fixture.keys);
  });

  it('independently accepts the frozen built-in pack', () => {
    const bytes = readFileSync(
      path.resolve(process.cwd(), '../../../contracts/builtin-props-v1.tmtprop.json')
    );
    const length = Buffer.alloc(8);
    length.writeBigUInt64BE(BigInt(bytes.length));
    const digest = createHash('sha256')
      .update(Buffer.from('TMT-OFFICE-PROP-PACK-V1\0'))
      .update(length)
      .update(bytes)
      .digest('hex');
    expect(`sha256:${digest}`).toBe(BUILTIN_DIGEST);
    expect(decodePropPack(builtinDocument)).toEqual(BUILTIN_PACK);
    expect(indexedProp(BUILTIN_PACK, 'desk')?.footprint).toEqual({ width: 4, height: 2 });
  });

  it('rejects executable-looking fields and unknown palette indexes', () => {
    expect(() => decodePropPack({ ...builtinDocument, script: 'throw 1' })).toThrow();
    const invalid = structuredClone(builtinDocument);
    invalid.props[0]!.pixels[0] = 'f'.repeat(invalid.props[0]!.pixels[0]!.length);
    expect(() => decodePropPack(invalid)).toThrow('Invalid prop pixels.');
  });

  it.each(vectors.packCases)('$name', ({ valid, value }) => {
    if (valid) expect(() => decodePropPack(value)).not.toThrow();
    else expect(() => decodePropPack(value)).toThrow();
  });

  it('accepts the shared maximum prop and pixel capacity', () => {
    const { capacity } = vectors;
    const props = Array.from({ length: capacity.count }, (_, index) => ({
      ...capacity.prop,
      key: `${capacity.keyPrefix}${String(index).padStart(2, '0')}`,
      label: capacity.label,
      pixels: Array(capacity.rasterRows).fill(capacity.rasterRow),
    }));
    expect(decodePropPack({ ...capacity.pack, props }).props).toHaveLength(capacity.count);
  });
});
