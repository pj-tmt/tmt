import builtinDocument from '../../../../../../../contracts/office/builtin-props-v1.tmtprop.json' with { type: 'json' };
import directionalWORKSTATIONDocument from '../../../../../../../contracts/office/directional-workstation-v2.tmtprop.json' with { type: 'json' };
import directionalLOUNGEDocument from '../../../../../../../contracts/office/directional-lounge-v2.tmtprop.json' with { type: 'json' };
import directionalRECEPTIONDocument from '../../../../../../../contracts/office/directional-reception-v2.tmtprop.json' with { type: 'json' };
import directionalFACILITIESDocument from '../../../../../../../contracts/office/directional-facilities-v2.tmtprop.json' with { type: 'json' };
import workshopDocument from '../../../../../../../contracts/office/workshop-furniture-v2.tmtprop.json' with { type: 'json' };
import commonsDocument from '../../../../../../../contracts/office/commons-props-v2.tmtprop.json' with { type: 'json' };
import whiteboardDocument from '../../../../../../../contracts/office/whiteboard-props-v2.tmtprop.json' with { type: 'json' };
import broadcasterDocument from '../../../../../../../contracts/office/broadcaster-props-v2.tmtprop.json' with { type: 'json' };
import studyDocument from '../../../../../../../contracts/office/study-furniture-v2.tmtprop.json' with { type: 'json' };
import wallDocument from '../../../../../../../contracts/office/wall-props-v2.tmtprop.json' with { type: 'json' };
import workstationDocument from '../../../../../../../contracts/office/modular-workstation-v2.tmtprop.json' with { type: 'json' };
import mountedDocument from '../../../../../../../contracts/office/modular-mounted-v2.tmtprop.json' with { type: 'json' };
import loungeDocument from '../../../../../../../contracts/office/modular-lounge-v2.tmtprop.json' with { type: 'json' };
import facilitiesDocument from '../../../../../../../contracts/office/modular-facilities-v2.tmtprop.json' with { type: 'json' };
import receptionDocument from '../../../../../../../contracts/office/modular-reception-v2.tmtprop.json' with { type: 'json' };
import { decodePropCapabilities, permitsCustomization } from './prop-customization.js';
import type { PropCapabilities, PropCustomization } from './prop-customization.js';
import { boundedText, exactRecord } from '../contracts/record.js';
import { tintPalette } from '../rendering/indexed-art.js';
import {
  indexedArtKey,
  INDEXED_ART_LIMITS,
  indexedLicense,
  indexedPalette,
  indexedRaster,
} from '../rendering/indexed-art-contract.js';

export const BUILTIN_DIGEST =
  'sha256:5aa6a2d239d7111586abc06be799b2a1ec2ca46619752a90ae08a13e414afb6a';
export const WORKSHOP_DIGEST =
  'sha256:288fb4f9ef08db8bdf635fbd1b16a3d602fbe0d53a095d96a7988969dabe3529';
export const COMMONS_DIGEST =
  'sha256:39a02590febbe0b7e9175951db1d7908a9b66ef32aa37eeb1ed9aa5cd524f63c';
export const WHITEBOARD_DIGEST =
  'sha256:2514687c911f644e28ea816e2c28b611d2c074e0105e584e6bba86ca208797e8';
export const BROADCASTER_DIGEST =
  'sha256:00f2f262077a0486fb3f4524a4efb6125c64a4349faaada079b13f6b25067a04';
export const STUDY_DIGEST =
  'sha256:78f0c0dc0700aaa37c55ae8cbe91c2d96585555a06e093a9131b529791360eed';
export const WALL_DIGEST =
  'sha256:5303fe9a3e5bf8a22c9958faeef1922a3cc21cfefb95a7b701a6a86213ac4415';
export const PROP_FOOTPRINT_LIMIT = 16;
export const DIRECTIONAL_WORKSTATION_DIGEST =
  'sha256:510f5c18585f9c626260ca7d851c10df9ee1f6858e2dedb494aff8dc7ad82003';
export const DIRECTIONAL_LOUNGE_DIGEST =
  'sha256:79b890d1e7f7a9139e856a45efdbfb111052dd4b9414050367f8290f677ea802';
export const DIRECTIONAL_RECEPTION_DIGEST =
  'sha256:bbd2099aec2ebef001386e84bc28c7ff119cdecb2cb566665b51b1fabb99a93b';
export const DIRECTIONAL_FACILITIES_DIGEST =
  'sha256:206562d849112c6ad6bfe2099bd1dc62fcdb82c73f58c3ff7e6062dd9fcf924f';
export const MODULAR_WORKSTATION_DIGEST =
  'sha256:10dc14a38d1cb0c92148c084b5e6239a54ee401348070444ae65fe8f6d815755';
export const MODULAR_MOUNTED_DIGEST =
  'sha256:86e7784ccb08d6c8804de7deeb2e3063898d3804e6ed81e3f7c8735b1996c7eb';
export const MODULAR_RECEPTION_DIGEST =
  'sha256:a00df6330d569dd6ab8d94bab391c074306be6529fdd224d5da9f9ef601052dc';
export const MODULAR_LOUNGE_DIGEST =
  'sha256:a4538f15b7da963679094f89d6f954215453492b5eb23bde40a4ffc6969a64b2';
export const MODULAR_FACILITIES_DIGEST =
  'sha256:b400ccadbacad373c9f420845a820256840829de7856f587cd5fd7d78786a55c';
export interface Footprint {
  width: number;
  height: number;
}
interface PropBase {
  key: string;
  label: string;
  footprint: Footprint;
  customization?: PropCapabilities;
}
export type PropDefinition = PropBase &
  (
    | { pixels: string[]; frames?: never }
    | { frames: [string[], string[], string[], string[]]; pixels?: never }
  );
export interface PropPack {
  formatVersion: 1 | 2;
  label: string;
  credit: string;
  license: string;
  palette: string[];
  props: PropDefinition[];
}
export interface CatalogPack {
  digest: string;
  pack: PropPack;
}

export const PROP_DIRECTIONS = ['South', 'West', 'North', 'East'] as const;

export function decodePropPack(value: unknown): PropPack {
  const object = exactRecord(
    value,
    ['formatVersion', 'label', 'credit', 'license', 'palette', 'props'],
    'prop pack'
  );
  const palette = object.palette;
  if (
    (object.formatVersion !== 1 && object.formatVersion !== 2) ||
    !boundedText(object.label, INDEXED_ART_LIMITS.labelBytes) ||
    !boundedText(object.credit, INDEXED_ART_LIMITS.creditBytes) ||
    !indexedLicense(object.license) ||
    !indexedPalette(palette, object.formatVersion === 2 ? 256 : 16) ||
    !Array.isArray(object.props) ||
    object.props.length < 1 ||
    object.props.length > 16
  )
    throw new Error('Invalid prop pack values.');
  const keys = new Set<string>();
  let totalPixels = 0;
  const directional = object.formatVersion === 2;
  const props = object.props.map((value) => {
    const prop = exactRecord(
      value,
      [
        'key',
        'label',
        'footprint',
        directional ? 'frames' : 'pixels',
        ...(directional &&
        value &&
        typeof value === 'object' &&
        Object.hasOwn(value, 'customization')
          ? ['customization']
          : []),
      ],
      'prop definition'
    );
    const footprint = exactRecord(prop.footprint, ['width', 'height'], 'prop footprint');
    if (
      !indexedArtKey(prop.key) ||
      keys.has(prop.key) ||
      !boundedText(prop.label, INDEXED_ART_LIMITS.labelBytes) ||
      !Number.isInteger(footprint.width) ||
      !Number.isInteger(footprint.height) ||
      (footprint.width as number) < 1 ||
      (footprint.width as number) > (directional ? PROP_FOOTPRINT_LIMIT : 8) ||
      (footprint.height as number) < 1 ||
      (footprint.height as number) > (directional ? PROP_FOOTPRINT_LIMIT : 8)
    )
      throw new Error('Invalid prop definition.');
    keys.add(prop.key);
    const base = {
      key: prop.key,
      label: prop.label,
      footprint: { width: footprint.width as number, height: footprint.height as number },
    };
    const decode = (value: unknown) => {
      const pixels = indexedRaster(value, palette.length, {
        maxSide: directional ? 128 : 64,
        maxCells: directional ? 16_384 : 4096,
        indexWidth: directional ? 2 : 1,
      });
      if (!pixels) throw new Error('Invalid prop pixels.');
      if (directional && !pixels.some((row) => /[1-9a-f]/.test(row)))
        throw new Error('Directional prop frames must contain visible pixels.');
      totalPixels += (pixels[0]!.length / (directional ? 2 : 1)) * pixels.length;
      return pixels;
    };
    if (!directional) return { ...base, pixels: decode(prop.pixels) };
    if (!Array.isArray(prop.frames) || prop.frames.length !== 4)
      throw new Error('Directional props require four frames.');
    const frames: [string[], string[], string[], string[]] = [
      decode(prop.frames[0]),
      decode(prop.frames[1]),
      decode(prop.frames[2]),
      decode(prop.frames[3]),
    ];
    return {
      ...base,
      frames,
      ...(Object.hasOwn(prop, 'customization')
        ? { customization: decodePropCapabilities(prop.customization, frames, palette.length) }
        : {}),
    };
  });
  if (totalPixels > (directional ? 131_072 : 65_536))
    throw new Error('Prop pack pixels exceed their bound.');
  return {
    formatVersion: object.formatVersion,
    label: object.label,
    credit: object.credit,
    license: object.license,
    palette,
    props,
  };
}

export const BUILTIN_PACK = decodePropPack(builtinDocument);
export const WORKSHOP_FURNITURE: CatalogPack = {
  digest: WORKSHOP_DIGEST,
  pack: decodePropPack(workshopDocument),
};
export const COMMONS_PROPS: CatalogPack = {
  digest: COMMONS_DIGEST,
  pack: decodePropPack(commonsDocument),
};
export const STUDY_FURNITURE: CatalogPack = {
  digest: STUDY_DIGEST,
  pack: decodePropPack(studyDocument),
};
export const MODULAR_WORKSTATION: CatalogPack = {
  digest: MODULAR_WORKSTATION_DIGEST,
  pack: decodePropPack(workstationDocument),
};
export const MODULAR_MOUNTED: CatalogPack = {
  digest: MODULAR_MOUNTED_DIGEST,
  pack: decodePropPack(mountedDocument),
};
export const BUILTIN_CATALOG: readonly CatalogPack[] = [
  { digest: BUILTIN_DIGEST, pack: BUILTIN_PACK },
  WORKSHOP_FURNITURE,
  COMMONS_PROPS,
  { digest: WHITEBOARD_DIGEST, pack: decodePropPack(whiteboardDocument) },
  { digest: BROADCASTER_DIGEST, pack: decodePropPack(broadcasterDocument) },
  STUDY_FURNITURE,
  { digest: WALL_DIGEST, pack: decodePropPack(wallDocument) },
  MODULAR_WORKSTATION,
  MODULAR_MOUNTED,
  { digest: MODULAR_LOUNGE_DIGEST, pack: decodePropPack(loungeDocument) },
  { digest: MODULAR_FACILITIES_DIGEST, pack: decodePropPack(facilitiesDocument) },
  { digest: MODULAR_RECEPTION_DIGEST, pack: decodePropPack(receptionDocument) },
  { digest: DIRECTIONAL_WORKSTATION_DIGEST, pack: decodePropPack(directionalWORKSTATIONDocument) },
  { digest: DIRECTIONAL_LOUNGE_DIGEST, pack: decodePropPack(directionalLOUNGEDocument) },
  { digest: DIRECTIONAL_RECEPTION_DIGEST, pack: decodePropPack(directionalRECEPTIONDocument) },
  { digest: DIRECTIONAL_FACILITIES_DIGEST, pack: decodePropPack(directionalFACILITIESDocument) },
];

/** Select admitted art once; both canvas and SVG consume this projection. */
export function propFrame(
  pack: PropPack,
  prop: PropDefinition,
  rotation = 0,
  customization?: PropCustomization
) {
  if (!Number.isInteger(rotation) || rotation < 0 || rotation > 3)
    throw new Error('Invalid prop rotation.');
  const directional = prop.frames !== undefined;
  if (!permitsCustomization(prop.customization, customization))
    throw new Error('Unsupported prop customization.');
  const palette =
    customization?.tint && prop.customization?.tint
      ? tintPalette(pack.palette, prop.customization.tint.indices, customization.tint)
      : [...pack.palette];
  const text =
    customization?.text && prop.customization?.text
      ? {
          value: customization.text,
          color: prop.customization.text.color,
          ...prop.customization.text.regions[rotation]!,
        }
      : undefined;
  return {
    pixels: directional ? prop.frames[rotation]! : prop.pixels,
    palette,
    ...(text ? { text } : {}),
    indexWidth: directional ? (2 as const) : (1 as const),
    width: directional && rotation % 2 ? prop.footprint.height : prop.footprint.width,
    height: directional && rotation % 2 ? prop.footprint.width : prop.footprint.height,
    rotation: directional ? 0 : rotation,
    frame: directional ? rotation : 0,
  };
}

export function indexedProp(pack: PropPack, key: string): PropDefinition | undefined {
  return pack.props.find((prop) => prop.key === key);
}

export function resolvedProp(
  pack: PropPack,
  key: string,
  footprint: Footprint,
  customization?: PropCustomization
): PropDefinition | undefined {
  const prop = indexedProp(pack, key);
  return prop &&
    prop.footprint.width === footprint.width &&
    prop.footprint.height === footprint.height &&
    permitsCustomization(prop.customization, customization)
    ? prop
    : undefined;
}

/** One capability-aware resolution path for scene pixels, placeholders and controls. */
export function resolvePlacedProp(
  catalog: readonly CatalogPack[],
  item: { prop: string; footprint: Footprint; customization?: PropCustomization }
) {
  const [digest, key] = item.prop.split('/');
  const pack = catalog.find((entry) => entry.digest === digest)?.pack;
  const definition =
    pack && key ? resolvedProp(pack, key, item.footprint, item.customization) : undefined;
  return pack && definition ? { pack, definition } : undefined;
}
