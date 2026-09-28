import { useState } from 'react';
import type { CatalogPack } from '../props/prop-contract.js';
import { PropThumbnail } from '../props/prop-thumbnail.js';
import type { CatalogDragHandlers } from './use-catalog-drag.js';
import { BUILTIN_DIGEST } from '../props/prop-contract.js';
import { directionalFurniture } from '../props/furniture-upgrades.js';

/** Browse the admitted catalog; choosing art still belongs to the world draft. */
export function WorldArtLibrary({
  catalog,
  choose,
  drag,
}: {
  catalog: CatalogPack[];
  choose: (pack: CatalogPack, key: string) => void;
  drag?: CatalogDragHandlers;
}) {
  const [query, setQuery] = useState('');
  const terms = query.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
  const groups = catalog
    .filter((entry) => entry.digest !== BUILTIN_DIGEST)
    .map((entry) => ({
      entry,
      props: entry.pack.props.filter((prop) => {
        const placement = {
          prop: `${entry.digest}/${prop.key}`,
          footprint: prop.footprint,
          x: 0,
          y: 0,
          rotation: 0,
        };
        if (directionalFurniture(placement, catalog).prop !== placement.prop) return false;
        const text = `${entry.pack.label} ${prop.label}`.toLocaleLowerCase();
        return terms.every((term) => text.includes(term));
      }),
    }))
    .filter((group) => group.props.length > 0);
  return (
    <>
      <label>
        Search objects
        <input
          type="search"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder="Try desk, plant or window"
        />
      </label>
      {groups.length === 0 && (
        <div role="status">
          <p>No matching objects.</p>
          <button onClick={() => setQuery('')}>Clear search</button>
        </div>
      )}
      {groups.map(({ entry, props }) => (
        <div key={entry.digest}>
          <h4>{entry.pack.label}</h4>
          <div className="world-art-options">
            {props.map((prop) => (
              <button
                key={prop.key}
                className="world-art-option"
                onPointerDown={(event) => drag?.start(event, entry, prop.key)}
                onDragStart={(event) => event.preventDefault()}
                onClick={(event) => {
                  if (!drag?.consumeClick(event.detail)) choose(entry, prop.key);
                }}
              >
                <PropThumbnail pack={entry.pack} prop={prop} />
                <span>{prop.label}</span>
              </button>
            ))}
          </div>
        </div>
      ))}
    </>
  );
}
