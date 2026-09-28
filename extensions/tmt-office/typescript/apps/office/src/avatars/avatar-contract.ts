import { boundedText, exactRecord } from '../contracts/record.js';
import type { AvatarArt } from '../profiles/avatar-art.js';
import {
  indexedArtKey,
  INDEXED_ART_LIMITS,
  indexedLicense,
  indexedPalette,
  indexedRaster,
} from '../rendering/indexed-art-contract.js';

export interface AvatarDefinition {
  key: string;
  label: string;
  pixels: string[];
}

export interface AvatarPack {
  formatVersion: 1 | 2;
  label: string;
  credit: string;
  license: string;
  palette: string[];
  avatars: AvatarDefinition[];
}

export interface CatalogAvatarPack {
  digest: string;
  pack: AvatarPack;
}

const FORMATS = {
  1: { width: 16, height: 24, indexWidth: 1, paletteLimit: 16 },
  2: { width: 32, height: 48, indexWidth: 2, paletteLimit: 256 },
} as const;

/** Preview and catalog resolution must preserve the admitted encoding together. */
export function avatarRaster(pack: AvatarPack, avatar: AvatarDefinition): AvatarArt {
  return {
    pixels: avatar.pixels,
    palette: pack.palette,
    indexWidth: FORMATS[pack.formatVersion].indexWidth,
  };
}

export function decodeAvatarPack(value: unknown): AvatarPack {
  const object = exactRecord(
    value,
    ['formatVersion', 'label', 'credit', 'license', 'palette', 'avatars'],
    'avatar pack'
  );
  const palette = object.palette;
  if (object.formatVersion !== 1 && object.formatVersion !== 2)
    throw new Error('Invalid avatar format version.');
  const format = FORMATS[object.formatVersion];
  if (
    !boundedText(object.label, INDEXED_ART_LIMITS.labelBytes) ||
    !boundedText(object.credit, INDEXED_ART_LIMITS.creditBytes) ||
    !indexedLicense(object.license) ||
    !indexedPalette(palette, format.paletteLimit) ||
    !Array.isArray(object.avatars) ||
    object.avatars.length < 1 ||
    object.avatars.length > 16
  )
    throw new Error('Invalid avatar pack values.');
  const keys = new Set<string>();
  let cellCount = 0;
  const avatars = object.avatars.map((value) => {
    const avatar = exactRecord(value, ['key', 'label', 'pixels'], 'avatar definition');
    if (
      !indexedArtKey(avatar.key) ||
      keys.has(avatar.key) ||
      !boundedText(avatar.label, INDEXED_ART_LIMITS.labelBytes)
    )
      throw new Error('Invalid avatar definition.');
    keys.add(avatar.key);
    const pixels = indexedRaster(avatar.pixels, palette.length, {
      width: format.width,
      height: format.height,
      maxSide: format.height,
      maxCells: format.width * format.height,
      indexWidth: format.indexWidth,
    });
    if (!pixels || !pixels.some((row) => /[1-9a-f]/.test(row)))
      throw new Error('Invalid avatar pixels.');
    cellCount += format.width * format.height;
    return { key: avatar.key, label: avatar.label, pixels };
  });
  if (cellCount > 6144) throw new Error('Avatar pack cells exceed their bound.');
  return {
    formatVersion: object.formatVersion,
    label: object.label,
    credit: object.credit,
    license: object.license,
    palette,
    avatars,
  };
}
