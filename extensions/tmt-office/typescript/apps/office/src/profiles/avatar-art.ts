import type { Appearance } from './profile-contract.js';

import type { IndexedArt } from '../rendering/indexed-art.js';
import { tintPalette } from '../rendering/indexed-art.js';
import { avatarRaster, decodeAvatarPack } from '../avatars/avatar-contract.js';
import robotDocument from '../../../../../../../contracts/office/workshop-robot-v2.tmtavatar.json' with { type: 'json' };
import materials from './robot-materials.json' with { type: 'json' };

export type AvatarArt = Pick<IndexedArt, 'pixels' | 'palette' | 'indexWidth'>;

const pack = decodeAvatarPack(robotDocument);
const body = avatarRaster(pack, pack.avatars[0]!);

// Existing accessory silhouettes use a two-pixel grid on the detailed shell.
// Zero preserves the generated body. The visor and antenna stay unobstructed.
const HAIRSTYLES: Record<Appearance['hairStyle'], readonly string[]> = {
  short: ['0000144004410000', '0001455445541000', '0014555555554100', '0014400000044100'],
  bob: [
    '0001144004411000',
    '0014555445554100',
    '0014555555554100',
    '0144000000004410',
    '0140000000000410',
    '0140000000000410',
    '0140000000000410',
    '0140000000000410',
    '0110000000000110',
  ],
  curls: [
    '0001410000141000',
    '0015451001545100',
    '0145454554545410',
    '0144100000014410',
    '0010000000000100',
  ],
  tied: [
    '0000144004410110',
    '0001455445541451',
    '0014555555551451',
    '0014400000041110',
    '0000000000001451',
    '0000000000001410',
    '0000000000000110',
  ],
  bald: [],
};

// Retain stored skinTone choices as robot shell finishes; no profile migration.
const SHELL = {
  light: '#f1edda',
  warm: '#e7c997',
  medium: '#99b79a',
  deep: '#849da5',
} as const;
const HAIR = {
  ink: ['#26313a', '#46545e'],
  brown: ['#684632', '#926445'],
  gold: ['#a77b32', '#d9b561'],
  silver: ['#7b8991', '#c0cbd0'],
} as const;
const SHIRT = {
  blue: '#8cc7df',
  green: '#95cda0',
  clay: '#efa483',
  plum: '#c4a3ce',
  gold: '#edcf85',
  ink: '#8d9da6',
} as const;

export function avatarArt(appearance: Appearance): AvatarArt {
  const overlay = HAIRSTYLES[appearance.hairStyle];
  const palette = tintPalette(
    tintPalette(body.palette, materials.shell, SHELL[appearance.skinTone]),
    materials.body,
    SHIRT[appearance.shirtColor]
  );
  for (const [offset, index] of materials.accessory.entries())
    palette[index] = HAIR[appearance.hairColor][offset] + 'ff';
  const accessory: Record<string, number> = {
    '1': materials.outline,
    '4': materials.accessory[0]!,
    '5': materials.accessory[1]!,
  };
  return {
    indexWidth: 2,
    pixels: body.pixels.map((row, y) =>
      row
        .match(/../g)!
        .map((pixel, x) => {
          const hair = overlay[Math.floor((y - 10) / 2)]?.[Math.floor(x / 2)];
          return hair && hair !== '0' ? accessory[hair]!.toString(16).padStart(2, '0') : pixel;
        })
        .join('')
    ),
    palette,
  };
}
