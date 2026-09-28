import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useState,
  useSyncExternalStore,
} from 'react';
import type { ReactNode } from 'react';
import {
  OBJECT_LIMIT,
  catalogFurniture,
  defaultCatalog,
  validFurniture,
} from './block-contract.js';
import type { BlockPort, Furniture } from './block-contract.js';
import { createBlockState } from './block-state.js';
import type { BlockState } from './block-state.js';
import { BUILTIN_CATALOG, resolvePlacedProp } from '../props/prop-contract.js';
import type { CatalogPack, PropDefinition } from '../props/prop-contract.js';
import { IndexedProp } from '../props/indexed-prop.js';
import { PropCustomizationFields } from '../props/prop-customization-fields.js';
import { BlockScene } from './block-scene.js';
import type { SceneAvatar, BlockSceneProps } from './block-scene.js';
import './block.css';

export const BlockContext = createContext<BlockPort | undefined>(undefined);
const DEFAULT_FURNITURE = [BUILTIN_CATALOG[0]!];
type StarterLayout = {
  id: string;
  label: string;
  description: string;
  create: () => Furniture[];
};
export function BlockPanel({
  blockKey,
  blockPort,
  label,
  avatar,
  renderScene,
  furniturePacks,
  editing = true,
  toolsId,
  starterLayouts,
}: {
  blockKey: string;
  blockPort?: BlockPort;
  label?: string;
  avatar?: SceneAvatar;
  renderScene?: (props: BlockSceneProps) => ReactNode;
  furniturePacks?: readonly CatalogPack[];
  editing?: boolean;
  toolsId?: string;
  starterLayouts?: readonly StarterLayout[];
}) {
  const defaultPort = useContext(BlockContext);
  const port = blockPort ?? defaultPort;
  const [state, setState] = useState<BlockState>();
  useEffect(() => {
    if (!port) return;
    const next = createBlockState(port, blockKey);
    setState(next);
    return () => next.dispose();
  }, [port, blockKey]);
  return state ? (
    <BlockEditor
      state={state}
      label={label}
      avatar={avatar}
      renderScene={renderScene}
      furniturePacks={furniturePacks}
      editing={editing}
      toolsId={toolsId}
      starterLayouts={starterLayouts}
    />
  ) : null;
}
export function BlockEditor({
  state,
  label = 'YOUR SPACE / HOME BLOCK',
  avatar,
  renderScene = (props) => <BlockScene {...props} />,
  furniturePacks = DEFAULT_FURNITURE,
  editing = true,
  toolsId,
  starterLayouts,
}: {
  state: BlockState;
  label?: string;
  avatar?: SceneAvatar;
  renderScene?: (props: BlockSceneProps) => ReactNode;
  furniturePacks?: readonly CatalogPack[];
  editing?: boolean;
  toolsId?: string;
  starterLayouts?: readonly StarterLayout[];
}) {
  const { remote, draft, ready, busy, error } = useSyncExternalStore(
    state.subscribe,
    state.getSnapshot
  );
  const [selected, setSelected] = useState<number | null>(null);
  const [starterId, setStarterId] = useState('');
  const starter = starterLayouts?.find((recipe) => recipe.id === starterId) ?? starterLayouts?.[0];
  // Static admitted artwork must not be decoded again on every draft keystroke.
  const palette = useMemo(
    () =>
      furniturePacks.flatMap((catalogPack) =>
        catalogPack.pack.props.map((definition) => ({
          catalogPack,
          definition,
          preview: (
            <svg
              viewBox={`0 0 ${definition.footprint.width} ${definition.footprint.height}`}
              aria-hidden="true"
              focusable="false"
            >
              <IndexedProp pack={catalogPack.pack} prop={definition} />
            </svg>
          ),
        }))
      ),
    [furniturePacks]
  );
  const objects = useMemo(
    () => draft?.objects ?? remote?.objects ?? [],
    [draft?.objects, remote?.objects]
  );
  const item = selected === null ? undefined : objects[selected];
  const catalog = useMemo(() => remote?.catalog ?? defaultCatalog(), [remote?.catalog]);
  const resolved = item ? resolvePlacedProp(catalog, item) : undefined;
  function objectLabel(object: Furniture): string {
    return resolvePlacedProp(catalog, object)?.definition.label ?? 'Unavailable prop';
  }
  function update(changes: Partial<Furniture>) {
    if (!item) return;
    state.edit(
      objects.map((value, index) => (index === selected ? { ...value, ...changes } : value))
    );
  }
  function add(catalogPack: CatalogPack, definition: PropDefinition) {
    state.edit([...objects, catalogFurniture(catalogPack, definition.key, 14, 14)]);
    setSelected(objects.length);
  }
  function rotate() {
    if (item) update({ rotation: (item.rotation + 1) % 4 });
  }
  function remove() {
    state.edit(objects.filter((_, index) => index !== selected));
    setSelected(null);
  }
  function step(dx: number, dy: number) {
    if (!item || busy || !validFurniture({ ...item, x: item.x + dx, y: item.y + dy })) return;
    return () => update({ x: item.x + dx, y: item.y + dy });
  }
  return (
    <section className="block-editor" aria-label="Office block editor" data-editing={editing}>
      <div className="block-heading">
        <div>
          <p className="eyebrow">{label}</p>
          <h2>Make room for your team.</h2>
        </div>
      </div>
      {error && <p role="alert">{error}</p>}
      {ready && (
        <div className="block-workbench">
          {renderScene({
            objects,
            selected: editing ? selected : undefined,
            select: busy || !editing ? undefined : setSelected,
            move: busy || !editing ? undefined : (x, y) => update({ x, y }),
            avatar,
            catalog,
            actions:
              editing && item
                ? {
                    label: objectLabel(item),
                    left: step(-1, 0),
                    right: step(1, 0),
                    up: step(0, -1),
                    down: step(0, 1),
                    rotate: busy ? undefined : rotate,
                    remove: busy ? undefined : remove,
                  }
                : undefined,
          })}
          <aside
            id={toolsId}
            className="block-tools"
            aria-label="Furniture controls"
            hidden={!editing}
          >
            <h3>Small things, your space.</h3>
            <p>
              Add a piece, then select a tile to move it. Nothing is saved until you choose Save.
            </p>
            {starter && objects.length === 0 && (
              <div className="block-starter">
                <label>
                  Room style
                  <select
                    value={starter.id}
                    disabled={busy}
                    onChange={(event) => setStarterId(event.target.value)}
                  >
                    {starterLayouts!.map((recipe) => (
                      <option key={recipe.id} value={recipe.id}>
                        {recipe.label}
                      </option>
                    ))}
                  </select>
                </label>
                <button
                  disabled={busy}
                  onClick={() => {
                    state.edit(starter.create());
                    setSelected(null);
                  }}
                >
                  Try a furnished room
                </button>
                <p>{starter.description} Adjust the draft, then save.</p>
              </div>
            )}
            <div className="furniture-catalog">
              {palette.map(({ catalogPack, definition, preview }) => (
                <button
                  key={`${catalogPack.digest}/${definition.key}`}
                  disabled={busy || objects.length >= OBJECT_LIMIT}
                  onClick={() => add(catalogPack, definition)}
                >
                  {preview}
                  <span>Add {definition.label.toLowerCase()}</span>
                </button>
              ))}
            </div>
            <p>
              {objects.length} / {OBJECT_LIMIT} pieces · 32 × 32 tiles
            </p>
            <ol className="furniture-list">
              {objects.map((object, index) => (
                <li key={index}>
                  <button aria-pressed={selected === index} onClick={() => setSelected(index)}>
                    {objectLabel(object)} {index + 1}
                  </button>
                </li>
              ))}
            </ol>
            {item && (
              <fieldset disabled={busy}>
                <legend>Selected {objectLabel(item)}</legend>
                {resolved && (
                  <PropCustomizationFields
                    {...resolved}
                    value={item.customization}
                    change={(customization) => {
                      const next = { ...item };
                      if (customization) next.customization = customization;
                      else delete next.customization;
                      state.edit(
                        objects.map((object, index) => (index === selected ? next : object))
                      );
                    }}
                  />
                )}
                <label>
                  Tile X
                  <input
                    type="number"
                    min="0"
                    max="31"
                    value={item.x}
                    onChange={(event) => update({ x: Number(event.target.value) })}
                  />
                </label>
                <label>
                  Tile Y
                  <input
                    type="number"
                    min="0"
                    max="31"
                    value={item.y}
                    onChange={(event) => update({ y: Number(event.target.value) })}
                  />
                </label>
                <button onClick={rotate}>Rotate clockwise</button>
                <button onClick={remove}>Remove selected</button>
              </fieldset>
            )}
            <p className="block-note">
              Private to your office. Only the owner and an explicitly assigned agent can access
              this block.
            </p>
          </aside>
        </div>
      )}
      {(editing || draft || busy || !ready || error) && (
        <div className="block-statusbar" aria-label="Layout changes">
          <span role="status">
            {!ready
              ? error
                ? 'Block unavailable'
                : 'Connecting…'
              : busy
                ? 'Saving…'
                : draft
                  ? 'Unsaved changes'
                  : remote
                    ? remote.revision === 0
                      ? 'Default layout · not saved'
                      : `Saved · revision ${remote.revision}`
                    : 'No saved layout yet'}
          </span>
          {ready && (
            <div className="block-actions">
              <button
                className="block-save"
                disabled={busy || (!draft && remote?.revision !== 0)}
                onClick={() => void state.save()}
              >
                Save layout
              </button>
              <button
                disabled={busy || !draft}
                onClick={() => {
                  state.reset();
                  setSelected(null);
                }}
              >
                Load latest layout
              </button>
            </div>
          )}
        </div>
      )}
    </section>
  );
}
