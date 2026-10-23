/**
 * The one definition of the 04D-1 "Beam / Window" mark: a dark terminal panel in perspective (its
 * left edge taller, corners rounded) with a `>_` prompt, and behind its right edge an aurora light
 * volume that feathers into its rounded far edge, throws a soft halo and spills a teal band back
 * along the panel's right inner edge. `BrandMark` (React), the shell's rail (static hosts) and
 * `scripts/generate-brand.mjs` (the static-host copy, favicon and app icons) all serialise it here.
 * Colours are classes; `brand-mark.css` maps them to the mark-only `--tmt-ui-brand-mark-*` roles.
 * The masks and the sheen use plain white as light, not a colour role.
 *
 * Below 28 px the small form flattens the panel, keeps only the chevron, and drops the halo,
 * spill, rim and sheen, so it still reads at 16 px. Filters work in user space inside the 96 box.
 */
type Point = readonly [number, number];

/** A polygon whose corners are quadratic curves of the given radii (0 keeps a corner sharp). */
function rounded(points: readonly Point[], radii: readonly number[]): string {
  const n = points.length;
  const toward = ([ax, ay]: Point, [bx, by]: Point, k: number): Point => {
    const length = Math.hypot(bx - ax, by - ay);
    const t = Math.min(k, length / 2) / length;
    return [ax + (bx - ax) * t, ay + (by - ay) * t];
  };
  let d = '';
  for (let i = 0; i < n; i++) {
    const corner = points[i]!;
    const [sx, sy] = toward(corner, points[(i - 1 + n) % n]!, radii[i]!);
    const [ex, ey] = toward(corner, points[(i + 1) % n]!, radii[i]!);
    d += `${i ? 'L' : 'M'}${sx.toFixed(2)} ${sy.toFixed(2)}Q${corner[0]} ${corner[1]} `;
    d += `${ex.toFixed(2)} ${ey.toFixed(2)}`;
  }
  return `${d}Z`;
}

const FULL = {
  // Corners about 9% of the panel height; the beam's far corners match.
  panel: rounded(
    [
      [6, 10],
      [50, 24],
      [50, 80],
      [6, 88],
    ],
    [8, 6.5, 6.5, 8],
  ),
  beam: rounded(
    [
      [7, 10],
      [74, 22],
      [80, 28],
      [80, 80],
      [74, 86],
      [50, 80],
      [7, 70],
    ],
    [0, 7, 7, 7, 7, 0, 0],
  ),
  rim: 'M49.4 29 L49.4 75',
  prompt: 'M15.5 34 L23.5 40.5 L15.5 47 M26.5 49.5 L34 51.2',
};
const SMALL = {
  panel: rounded(
    [
      [5, 14],
      [52, 24],
      [52, 76],
      [5, 86],
    ],
    [7, 7, 7, 7],
  ),
  beam: rounded(
    [
      [6, 14],
      [86, 24],
      [92, 29],
      [92, 81],
      [86, 86],
      [52, 76],
      [6, 70],
    ],
    [0, 9, 9, 9, 9, 0, 0],
  ),
  prompt: 'M15 37 L28 50 L15 63',
};

/** The small form below this rendered size. */
export const brandMarkSmallBelow = 28;
export const brandMarkSmall = (size: number | undefined): boolean =>
  size !== undefined && size < brandMarkSmallBelow;

const stop = (name: string, offset: number, opacity?: number) =>
  `<stop class="tmt-brand-mark-${name}" offset="${offset}"` +
  `${opacity === undefined ? '' : ` stop-opacity="${opacity}"`}></stop>`;
const light = (offset: number, opacity: number) =>
  `<stop offset="${offset}" stop-color="#fff" stop-opacity="${opacity}"></stop>`;
const box = 'filterUnits="userSpaceOnUse" x="0" y="0" width="96" height="96"';

/**
 * A paint-server id prefix that no element of the document uses yet: `base`, then `base-1`,
 * `base-2`. Marks that share a prefix resolve `url(#…)` to the first paint server in the document,
 * which can sit in a hidden subtree and paint nothing. `taken` asks the live document, so marks
 * drawn by separate bundles do not collide either.
 */
export function freshMarkId(base: string, taken: (prefix: string) => boolean): string {
  let id = base;
  for (let count = 1; taken(id); count++) id = `${base}-${count}`;
  return id;
}

/** The mark's children; `id` prefixes its paint servers, unique per document. */
export function brandMarkInner(small: boolean, id: string): string {
  const shape = small ? SMALL : FULL;
  const aurora =
    `<linearGradient id="${id}-aurora" x1="0.2" y1="0.05" x2="0.85" y2="1">` +
    `${stop('cyan', 0)}${stop('green', 0.5)}${stop('gold', 1)}</linearGradient>` +
    `<linearGradient id="${id}-far" x1="0" y1="0" x2="1" y2="0">` +
    `${stop('far', 0.45, 0)}${stop('far', 1, small ? 0.15 : 0.25)}</linearGradient>` +
    `<linearGradient id="${id}-panel" x1="0" y1="0" x2="0.6" y2="1">` +
    `${stop('panel-top', 0)}${stop('panel-bottom', 1)}</linearGradient>` +
    `<linearGradient id="${id}-mono" x1="0" y1="0" x2="1" y2="0.35">` +
    `${stop('mono-start', 0)}${stop('mono-end', 1)}</linearGradient>`;
  const beam =
    `<path d="${shape.beam}" fill="url(#${id}-aurora)"></path>` +
    `<path d="${shape.beam}" fill="url(#${id}-far)"></path>`;
  const print = `<path class="tmt-brand-mark-print" d="${shape.beam}" fill="url(#${id}-mono)"></path>`;
  const panel = `<path class="tmt-brand-mark-panel" d="${shape.panel}" fill="url(#${id}-panel)"></path>`;
  const prompt = `<path class="tmt-brand-mark-prompt" d="${shape.prompt}"></path>`;
  if (small)
    return `<defs>${aurora}</defs><g class="tmt-brand-mark-beam">${beam}</g>${print}${panel}${prompt}`;
  return (
    `<defs>${aurora}` +
    // The halo is strongest at the far edge and the bottom.
    `<linearGradient id="${id}-halo-weight" x1="0" y1="0" x2="1" y2="1">` +
    `${light(0.3, 0.15)}${light(1, 1)}</linearGradient>` +
    `<mask id="${id}-halo-mask" maskUnits="userSpaceOnUse" x="0" y="0" width="96" height="96">` +
    `<rect width="96" height="96" fill="url(#${id}-halo-weight)"></rect></mask>` +
    // The beam feathers into its far edge instead of ending in a hard line.
    `<linearGradient id="${id}-feather" x1="0" y1="0" x2="1" y2="0">` +
    `${light(0.88, 1)}${light(1, 0.7)}</linearGradient>` +
    `<mask id="${id}-feather-mask" maskContentUnits="objectBoundingBox">` +
    `<rect width="1" height="1" fill="url(#${id}-feather)"></rect></mask>` +
    // The light washing back as a band along the panel's right inner edge.
    `<linearGradient id="${id}-spill" gradientUnits="userSpaceOnUse" x1="50" y1="0" x2="36" y2="0">` +
    `${stop('glow', 0, 0.6)}${stop('cyan', 0.35, 0.22)}${stop('cyan', 1, 0)}</linearGradient>` +
    `<linearGradient id="${id}-sheen" x1="0" y1="0" x2="1" y2="1">` +
    `${light(0, 0.1)}${light(0.35, 0)}</linearGradient>` +
    `<linearGradient id="${id}-rim" x1="0" y1="0" x2="0" y2="1">` +
    `${stop('cyan', 0, 0.9)}${stop('gold', 1, 0.9)}</linearGradient>` +
    `<clipPath id="${id}-panel-clip"><path d="${shape.panel}"></path></clipPath>` +
    `<filter id="${id}-halo" ${box}><feGaussianBlur stdDeviation="2.4"></feGaussianBlur></filter>` +
    `<filter id="${id}-soft" ${box}><feGaussianBlur stdDeviation="0.25"></feGaussianBlur></filter>` +
    `</defs>` +
    // Scaled in so the halo stays inside the 96 box.
    `<g transform="translate(6.5 3) scale(0.94)">` +
    `<g class="tmt-brand-mark-beam">` +
    `<g mask="url(#${id}-halo-mask)"><path d="${shape.beam}" fill="url(#${id}-aurora)" ` +
    `filter="url(#${id}-halo)" opacity="0.55" transform="translate(1 1.4)"></path></g>` +
    `<g mask="url(#${id}-feather-mask)" filter="url(#${id}-soft)">${beam}</g></g>` +
    print +
    panel +
    `<g class="tmt-brand-mark-beam" clip-path="url(#${id}-panel-clip)">` +
    `<path d="${shape.panel}" fill="url(#${id}-spill)"></path>` +
    `<path class="tmt-brand-mark-rim" d="${FULL.rim}" stroke="url(#${id}-rim)"></path></g>` +
    `<path d="${shape.panel}" fill="url(#${id}-sheen)"></path>` +
    prompt +
    `</g>`
  );
}

/** The whole decorative `<svg>`, as static hosts embed it. */
export function brandMarkSvg({
  size,
  className = 'tmt-brand-mark',
  id,
}: {
  size?: number;
  className?: string;
  id: string;
}): string {
  const small = brandMarkSmall(size);
  const dimensions = size === undefined ? '' : ` width="${size}" height="${size}"`;
  return (
    `<svg class="${className}" data-form="${small ? 'small' : 'full'}" viewBox="0 0 96 96" ` +
    `aria-hidden="true"${dimensions}>${brandMarkInner(small, id)}</svg>`
  );
}

/** The app icon and favicon: the mark on the dark rounded-square tile; `small` below 48 px. */
export function brandTileInner(small: boolean, id: string): string {
  return (
    `<rect class="tmt-brand-mark-tile" x="0.5" y="0.5" width="95" height="95" rx="22"></rect>` +
    `<g class="tmt-brand-mark" data-form="${small ? 'small' : 'full'}" ` +
    `transform="translate(16 16) scale(0.6667)">${brandMarkInner(small, id)}</g>`
  );
}
