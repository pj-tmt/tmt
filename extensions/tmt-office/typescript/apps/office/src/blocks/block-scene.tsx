import { useId } from 'react';
import type { Furniture } from './block-contract.js';
import { BLOCK_SIZE, defaultCatalog, footprint } from './block-contract.js';
import { Avatar } from '../profiles/avatar.js';
import type { AvatarArt } from '../profiles/avatar-art.js';
import type { Appearance } from '../profiles/profile-contract.js';
import type { CatalogPack } from '../props/prop-contract.js';
import { resolvePlacedProp } from '../props/prop-contract.js';
import { IndexedProp } from '../props/indexed-prop.js';
import workshopFloor from './assets/workshop-oak-v1.png';

export interface SceneAvatar {
  appearance: Appearance;
  name: string;
  displayLabel?: string;
  customArt?: AvatarArt;
}

function ResolvedProp({ item, catalog }: { item: Furniture; catalog: CatalogPack[] }) {
  const clip = useId();
  const [digest] = item.prop.split('/');
  const digestPrefix = digest?.startsWith('sha256:')
    ? digest.slice('sha256:'.length, 'sha256:'.length + 12)
    : '000000000000';
  const resolved = resolvePlacedProp(catalog, item);
  const size = footprint(item);
  if (resolved)
    return (
      <g transform={`translate(${item.x} ${item.y})`}>
        <IndexedProp
          pack={resolved.pack}
          prop={resolved.definition}
          rotation={item.rotation}
          customization={item.customization}
        />
      </g>
    );
  return (
    <g
      transform={`translate(${item.x + size.width / 2} ${item.y + size.height / 2}) rotate(${item.rotation * 90}) translate(${-item.footprint.width / 2} ${-item.footprint.height / 2})`}
    >
      <defs>
        <clipPath id={clip}>
          <rect width={item.footprint.width} height={item.footprint.height} />
        </clipPath>
      </defs>
      <g role="img" aria-label={`Unavailable prop ${digestPrefix}`} clipPath={`url(#${clip})`}>
        <rect
          width={item.footprint.width}
          height={item.footprint.height}
          fill="var(--floor)"
          stroke="var(--selection)"
          strokeWidth="0.15"
          strokeDasharray="0.3 0.2"
        />
        <text x="0.25" y="0.65" fontSize="0.45" fill="var(--ink)">
          Unavailable prop
        </text>
        <text x="0.25" y="1.2" fontSize="0.32" fill="var(--ink)" opacity="0.65">
          {digestPrefix}
        </text>
      </g>
    </g>
  );
}

export interface BlockSceneProps {
  objects: Furniture[];
  selected?: number | null;
  select?: (index: number) => void;
  move?: (x: number, y: number) => void;
  avatar?: SceneAvatar;
  catalog?: CatalogPack[];
  actions?: FurnitureActions;
}

/** Commands supplied by the draft owner; renderers never mutate a layout. */
export interface FurnitureActions {
  label: string;
  left?: () => void;
  right?: () => void;
  up?: () => void;
  down?: () => void;
  rotate?: () => void;
  remove?: () => void;
}

export function BlockScene({
  objects,
  selected,
  select,
  move,
  avatar,
  catalog = defaultCatalog(),
}: BlockSceneProps) {
  const pattern = useId();
  const floor = useId();
  return (
    <svg
      className="block-scene"
      viewBox={`-1 -1 ${BLOCK_SIZE + 2} ${BLOCK_SIZE + 2}`}
      role="group"
      aria-label={
        move
          ? 'Office layout, 32 by 32 tiles. Use the furniture controls to edit.'
          : 'Office room, 32 by 32 tiles.'
      }
      onClick={(event) => {
        if (!move) return;
        const matrix = event.currentTarget.getScreenCTM();
        if (!matrix) return;
        const point = new DOMPoint(event.clientX, event.clientY).matrixTransform(matrix.inverse());
        move(Math.floor(point.x), Math.floor(point.y));
      }}
    >
      <defs>
        <pattern id={floor} width="32" height="32" patternUnits="userSpaceOnUse">
          <image href={workshopFloor} width="32" height="32" preserveAspectRatio="none" />
        </pattern>
        <pattern id={pattern} width="1" height="1" patternUnits="userSpaceOnUse">
          <path d="M1 0 H0 V1" fill="none" stroke="var(--grid)" strokeWidth="0.035" />
        </pattern>
      </defs>
      <rect x="-0.6" y="-0.6" width="33.2" height="33.2" rx="0.5" fill="var(--wall)" />
      <rect width="32" height="32" fill={`url(#${floor})`} />
      <rect width="32" height="32" fill="#eadcc0" opacity="0.22" />
      {move && <rect width="32" height="32" fill={`url(#${pattern})`} />}
      {objects.map((item, index) => {
        const { width, height } = footprint(item);
        return (
          <g
            key={index}
            onClick={(event) => {
              if (!select) return;
              event.stopPropagation();
              select(index);
            }}
          >
            <ResolvedProp item={item} catalog={catalog} />
            {selected === index && (
              <rect
                x={item.x - 0.1}
                y={item.y - 0.1}
                width={width + 0.2}
                height={height + 0.2}
                fill="none"
                stroke="var(--selection)"
                strokeWidth="0.12"
                strokeDasharray="0.25 0.15"
              />
            )}
          </g>
        );
      })}
      {avatar && (
        <Avatar
          appearance={avatar.appearance}
          name={avatar.name}
          displayLabel={avatar.displayLabel}
          customArt={avatar.customArt}
          x={24}
          y={25}
          scale={0.8}
        />
      )}
    </svg>
  );
}
