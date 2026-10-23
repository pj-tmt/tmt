import type { ReactNode } from 'react';
import { browserUiClasses as c } from './static.js';
import type { BrowserStatusTone } from './static.js';

/** A row of status tiles; each tile is a term and its value, so the group reads as a list. */
export function BrowserStatusTiles({ children }: { children: ReactNode }) {
  return <dl className={c.statusTiles}>{children}</dl>;
}

export interface BrowserStatusTileProps {
  label: string;
  value: string;
  /** The dot's colour; the value text carries the meaning. */
  tone?: BrowserStatusTone;
  /** The value is a count: set large in the display face, muted at `0` so a row of tiles keeps its shape. */
  numeric?: boolean;
}

/** One read-only fact: a small label over a dot and a value, such as `Page` over `● Available`. */
export function BrowserStatusTile({
  label,
  value,
  tone = 'muted',
  numeric,
}: BrowserStatusTileProps) {
  return (
    <div className={c.statusTile} data-tone={tone}>
      <dt className={c.statusTileLabel}>{label}</dt>
      <dd
        className={c.statusTileValue}
        data-numeric={numeric ? (value === '0' ? 'zero' : 'count') : undefined}
      >
        <span className={c.statusTileDot} aria-hidden="true" />
        {value}
      </dd>
    </div>
  );
}
