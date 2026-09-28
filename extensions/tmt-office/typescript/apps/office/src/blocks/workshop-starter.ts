import { catalogFurniture } from './block-contract.js';
import type { Furniture } from './block-contract.js';
import { WORKSHOP_FURNITURE, STUDY_FURNITURE } from '../props/prop-contract.js';
import type { CatalogPack } from '../props/prop-contract.js';

type Placement = [CatalogPack, string, number, number];
type RoomStyle = 'study' | 'studio' | 'library';

// List order is paint order: textiles, back furniture, then nearer furniture
// and desk equipment. Each recipe leaves the resident and doorway unobstructed.
const RECIPES: Record<RoomStyle, Placement[]> = {
  study: [
    [WORKSHOP_FURNITURE, 'woven-rug', 3, 10],
    [WORKSHOP_FURNITURE, 'woven-rug', 14, 0],
    [STUDY_FURNITURE, 'oak-bookcase', 0, 0],
    [STUDY_FURNITURE, 'reading-lamp', 10, 1],
    [WORKSHOP_FURNITURE, 'green-chair', 7, 10],
    [WORKSHOP_FURNITURE, 'oak-desk', 4, 13],
    [WORKSHOP_FURNITURE, 'lounge-sofa', 18, 1],
    [WORKSHOP_FURNITURE, 'coffee-table', 20, 7],
    [WORKSHOP_FURNITURE, 'leafy-plant', 1, 25],
    [WORKSHOP_FURNITURE, 'leafy-plant', 26, 11],
    [STUDY_FURNITURE, 'desktop-terminal', 6, 11],
  ],
  studio: [
    [WORKSHOP_FURNITURE, 'woven-rug', 4, 7],
    [STUDY_FURNITURE, 'reading-lamp', 0, 1],
    [STUDY_FURNITURE, 'oak-bookcase', 18, 0],
    [WORKSHOP_FURNITURE, 'green-chair', 10, 7],
    [WORKSHOP_FURNITURE, 'oak-desk', 7, 10],
    [WORKSHOP_FURNITURE, 'leafy-plant', 0, 15],
    [WORKSHOP_FURNITURE, 'leafy-plant', 12, 24],
    [STUDY_FURNITURE, 'desktop-terminal', 9, 8],
  ],
  library: [
    [WORKSHOP_FURNITURE, 'woven-rug', 1, 10],
    [WORKSHOP_FURNITURE, 'woven-rug', 14, 8],
    [STUDY_FURNITURE, 'oak-bookcase', 0, 0],
    [STUDY_FURNITURE, 'oak-bookcase', 18, 0],
    [STUDY_FURNITURE, 'reading-lamp', 11, 1],
    [WORKSHOP_FURNITURE, 'lounge-sofa', 18, 12],
    [WORKSHOP_FURNITURE, 'green-chair', 6, 10],
    [WORKSHOP_FURNITURE, 'oak-desk', 3, 13],
    [WORKSHOP_FURNITURE, 'leafy-plant', 0, 24],
    [STUDY_FURNITURE, 'desktop-terminal', 5, 11],
  ],
};

/** Explicit draft recipes, not another default-layout or persistence layer. */
export function workshopStarter(style: RoomStyle = 'study'): Furniture[] {
  return RECIPES[style].map(([pack, key, x, y]) => catalogFurniture(pack, key, x, y));
}

export const WORKSHOP_STARTERS = [
  {
    id: 'study',
    label: 'Cozy study',
    description: 'A writing desk, a lounge corner and greenery.',
    create: () => workshopStarter('study'),
  },
  {
    id: 'studio',
    label: 'Focus studio',
    description: 'A central desk, open floor and a quiet bookshelf.',
    create: () => workshopStarter('studio'),
  },
  {
    id: 'library',
    label: 'Reading room',
    description: 'Twin bookcases, a sofa and a tucked-away writing desk.',
    create: () => workshopStarter('library'),
  },
] as const;
