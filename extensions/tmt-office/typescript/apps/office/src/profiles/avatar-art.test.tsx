import { render } from '@testing-library/react';
import { expect, it } from 'vitest';
import { readRasterCells } from '../../../../../../../typescript/test/support/indexed-raster.js';
import { avatarArt } from './avatar-art.js';
import { Avatar } from './avatar.js';
import { AVATAR_LAYOUT, avatarMarkColor } from './avatar-layout.js';
import { avatarRaster, decodeAvatarPack } from '../avatars/avatar-contract.js';
import detailed from '../../../../../contracts/avatar-pack-v2-sample.tmtavatar.json';
import robot from '../../../../../contracts/workshop-robot-v2.tmtavatar.json';
import materials from './robot-materials.json';
import { PROFILE_CATALOG } from './profile-contract.js';
import type { Appearance } from './profile-contract.js';

const appearance: Appearance = {
  hairStyle: 'short',
  hairColor: 'ink',
  skinTone: 'warm',
  shirtColor: 'blue',
  shirtMark: '',
};

it('keeps marks readable on light default plates and dark custom artwork', () => {
  const light = avatarArt(appearance);
  expect(avatarMarkColor(light)).toBe('#203e37');
  const dark = { pixels: ['11', '11'], palette: ['#00000000', '#102020ff'] };
  expect(avatarMarkColor(dark)).toBe('#fff8e4');
  expect(avatarMarkColor({ ...dark, pixels: ['0101', '0101'], indexWidth: 2 })).toBe('#fff8e4');
});

it('renders v2 high palette indices on the same physical footprint without stretching the grid', () => {
  const pack = decodeAvatarPack(detailed);
  const view = render(
    <svg>
      <Avatar
        appearance={appearance}
        name="Detail"
        customArt={avatarRaster(pack, pack.avatars[0]!)}
      />
    </svg>
  );
  const raster = view.container.querySelector('svg[shape-rendering="crispEdges"]')!;
  expect(raster.getAttribute('viewBox')).toBe('0 0 32 48');
  expect(raster.getAttribute('width')).toBe(String(AVATAR_LAYOUT.width));
  expect(raster.getAttribute('height')).toBe(String(AVATAR_LAYOUT.height));
  expect(readRasterCells(view.container)).toContainEqual(['12', '16', '#b3f6d9ff']);
});

it('keeps every supported appearance on the bounded detailed avatar grid', () => {
  const silhouettes = new Set<string>();
  const invalidRows: { appearance: string; row: number; pixels: string }[] = [];
  for (const hairStyle of PROFILE_CATALOG.hairStyles) {
    silhouettes.add(avatarArt({ ...appearance, hairStyle }).pixels.join(''));
    for (const hairColor of PROFILE_CATALOG.hairColors)
      for (const skinTone of PROFILE_CATALOG.skinTones)
        for (const shirtColor of PROFILE_CATALOG.shirtColors) {
          const art = avatarArt({ ...appearance, hairStyle, hairColor, skinTone, shirtColor });
          expect(art.indexWidth).toBe(2);
          expect(art.pixels).toHaveLength(48);
          // Check every cell without creating hundreds of thousands of matcher
          // objects. Retain the exact appearance and row for failure diagnosis.
          art.pixels.forEach((row, y) => {
            if (
              !/^[0-9a-f]{64}$/.test(row) ||
              row
                .match(/../g)!
                .some((index) => art.palette[Number.parseInt(index, 16)] === undefined)
            )
              invalidRows.push({
                appearance: [hairStyle, hairColor, skinTone, shirtColor].join('/'),
                row: y,
                pixels: row,
              });
          });
        }
  }
  expect(silhouettes.size).toBe(5);
  expect(invalidRows).toEqual([]);
});

it('isolates palette customization and never bakes identity text into pixels', () => {
  const original = avatarArt(appearance);
  for (const [field, value, indexes] of [
    ['skinTone', 'deep', materials.shell],
    ['hairColor', 'silver', materials.accessory],
    ['shirtColor', 'plum', materials.body],
  ] as const) {
    const changed = avatarArt({ ...appearance, [field]: value });
    expect(changed.pixels).toEqual(original.pixels);
    expect(
      changed.palette.flatMap((color, index) => (color === original.palette[index] ? [] : [index]))
    ).toEqual(indexes);
  }
  expect(avatarArt({ ...appearance, shirtMark: '<script>' })).toEqual(original);
});

it('preserves the admitted antenna and visor beneath every accessory silhouette', () => {
  for (const hairStyle of PROFILE_CATALOG.hairStyles) {
    const art = avatarArt({ ...appearance, hairStyle });
    expect(art.pixels.slice(0, 10)).toEqual(robot.avatars[0]!.pixels.slice(0, 10));
    for (let y = 18; y < 27; y++)
      expect(art.pixels[y]!.slice(20, 48)).toBe(robot.avatars[0]!.pixels[y]!.slice(20, 48));
  }
});

it('uses distinct material slots and actual detailed pixels, not an enlarged v1 sprite', () => {
  const art = avatarArt({ ...appearance, hairStyle: 'bald' });
  expect(art.palette.length).toBeGreaterThan(16);
  expect(art.palette.length).toBeLessThanOrEqual(256);
  expect(art.pixels).toEqual(robot.avatars[0]!.pixels);
  expect(art.pixels.some((row, y) => y % 2 === 0 && row !== art.pixels[y + 1])).toBe(true);
  const slots = [...materials.shell, ...materials.body, ...materials.accessory, materials.outline];
  expect(new Set(slots).size).toBe(slots.length);
  for (const index of slots) {
    expect(index).toBeGreaterThan(0);
    expect(index).toBeLessThan(art.palette.length);
  }
  const used = new Set(
    art.pixels.flatMap((row) => row.match(/../g)!.map((index) => parseInt(index, 16)))
  );
  for (const index of [...materials.shell, ...materials.body]) expect(used.has(index)).toBe(true);
  // The fixed ivory chest plate stays legible across shell/body recoloring.
  const chest = Number.parseInt(art.pixels[34]!.slice(32, 34), 16);
  expect(slots).not.toContain(chest);
  expect(art.palette[chest]).toBe(robot.palette[chest]);
});

it('can serialize default robot artwork through the same avatar-pack admission as custom art', () => {
  for (const hairStyle of PROFILE_CATALOG.hairStyles) {
    const art = avatarArt({ ...appearance, hairStyle });
    const pack = {
      formatVersion: 2,
      label: 'Workshop robot',
      credit: 'TMT',
      license: 'MIT',
      palette: art.palette,
      avatars: [{ key: hairStyle, label: hairStyle, pixels: art.pixels }],
    };
    expect(decodeAvatarPack(JSON.parse(JSON.stringify(pack)))).toEqual(pack);
  }
});

it('renders actual unit pixels instead of smooth character geometry and keeps text inert', () => {
  const view = render(
    <svg>
      <Avatar appearance={{ ...appearance, shirtMark: '<img>' }} name="Alice & team" />
    </svg>
  );
  expect(view.container.querySelectorAll('circle,image,script,img')).toHaveLength(0);
  expect(
    view.container.querySelectorAll('svg[shape-rendering="crispEdges"] foreignObject')
  ).toHaveLength(0);
  const pixels = readRasterCells(view.container);
  expect(pixels.length).toBeGreaterThan(100);
  expect(new Set(pixels.map(([x, y]) => `${x}:${y}`)).size).toBe(pixels.length);
  expect(view.container.querySelector('.avatar-name')?.textContent).toBe('Alice & team');
  expect(view.container.querySelector('.avatar-mark')?.textContent).toBe('<img>');
  expect((view.container.querySelector('.avatar-mark') as HTMLElement).style.color).toBe(
    'rgb(32, 62, 55)'
  );
  const mark = view.container.querySelector('.avatar-mark')!.parentElement!;
  const center = Number(mark.getAttribute('y')) + Number(mark.getAttribute('height')) / 2 + 2.8;
  expect(center).toBeCloseTo(AVATAR_LAYOUT.markCenterY);
  const raster = view.container.querySelector('svg[shape-rendering="crispEdges"]')!;
  expect(raster.getAttribute('viewBox')).toBe('0 0 32 48');
  expect(Number(raster.getAttribute('width'))).toBe(AVATAR_LAYOUT.width);
  expect(Number(raster.getAttribute('height'))).toBe(AVATAR_LAYOUT.height);
  view.unmount();
});
