import { useEffect, useState } from 'react';
import { PixelCanvas } from './pixel-canvas.js';
import { IndexedProp } from './indexed-prop.js';
import { newPixelDraft, pixelDraftPack, pixelHistory, PIXEL_SIZES } from './pixel-draft.js';
import type { PixelDraft, PixelSize } from './pixel-draft.js';
import type { CatalogPack, PropPack } from './prop-contract.js';
import type { PropCatalogPort } from './prop-catalog-contract.js';
import { usePixelCatalog } from './use-pixel-catalog.js';
import './pixel-workshop.css';

function ArtPreview({ pack }: { pack: PropPack }) {
  return (
    <div className="pixel-art-previews">
      {pack.props.map((prop) => (
        <figure key={prop.key}>
          <svg
            role="img"
            aria-label={`${prop.label} preview`}
            viewBox={`0 0 ${prop.footprint.width} ${prop.footprint.height}`}
          >
            <IndexedProp pack={pack} prop={prop} />
          </svg>
          <figcaption>
            {prop.label} · {prop.footprint.width} × {prop.footprint.height} tiles
          </figcaption>
        </figure>
      ))}
    </div>
  );
}

/** Reads one selected immutable pack, not every raster in the catalog. */
function LibraryArt({
  digest,
  port,
  canAdd,
  add,
}: {
  digest: string;
  port: PropCatalogPort;
  canAdd: boolean;
  add: (catalog: CatalogPack, key: string) => void;
}) {
  const [state, setState] = useState<CatalogPack | 'failed'>();
  useEffect(() => {
    const controller = new AbortController();
    void port.load(digest, controller.signal).then(
      (pack) => {
        if (!controller.signal.aborted) setState(pack);
      },
      () => {
        if (!controller.signal.aborted) setState('failed');
      }
    );
    return () => controller.abort();
  }, [digest, port]);
  if (state === 'failed')
    return (
      <p role="alert">This artwork could not load. Refresh the library and select it again.</p>
    );
  if (!state) return <p role="status">Loading artwork…</p>;
  return (
    <>
      <ArtPreview pack={state.pack} />
      <p>
        {state.pack.credit} · {state.pack.license}
      </p>
      {state.pack.props.map((prop) => (
        <button key={prop.key} disabled={!canAdd} onClick={() => add(state, prop.key)}>
          Add {prop.label} to layout draft
        </button>
      ))}
    </>
  );
}

export function PixelWorkshop({
  port,
  canAdd,
  add,
}: {
  port: PropCatalogPort;
  canAdd: boolean;
  add: (catalog: CatalogPack, key: string) => void;
}) {
  const [history, setHistory] = useState(() => pixelHistory.create(newPixelDraft()));
  const [color, setColor] = useState(3);
  const [drawing, setDrawing] = useState(false);
  const [selected, setSelected] = useState<string>();
  const [newSize, setNewSize] = useState<PixelSize>(16);
  const [confirmNew, setConfirmNew] = useState(false);
  const [placement, setPlacement] = useState<string>();
  const [view, setView] = useState<'create' | 'library'>('create');
  const catalog = usePixelCatalog(port);
  const draft = pixelHistory.current(history);
  let preview: PropPack | undefined;
  let invalid: string | undefined;
  try {
    preview = pixelDraftPack(draft);
  } catch (cause) {
    invalid = cause instanceof Error ? cause.message : 'Artwork is incomplete.';
  }
  const frozen = catalog.locked;
  function change(next: PixelDraft) {
    if (frozen) return;
    setHistory((current) => pixelHistory.commit(current, next));
    catalog.edited();
    setPlacement(undefined);
  }
  function place(pack: CatalogPack, key: string) {
    try {
      add(pack, key);
      setPlacement(
        'Added to the layout draft. Close the workshop to position it, then Save layout.'
      );
    } catch (cause) {
      setPlacement(cause instanceof Error ? cause.message : 'Could not add artwork.');
    }
  }
  function refresh(cursor?: string) {
    setSelected(undefined);
    void catalog.refresh(cursor);
  }
  return (
    <section className="pixel-workshop">
      <p className="eyebrow">LOCAL ART STUDIO</p>
      <h2>Pixel workshop</h2>
      <p>
        Draw a small artwork, save it to your library, then place it on the floor or a wall. Saving
        art does not change the office. Closing this panel keeps your drawing for this visit.
      </p>
      <nav className="pixel-toolbar" aria-label="Workshop views">
        <button
          aria-pressed={view === 'create'}
          disabled={drawing}
          onClick={() => setView('create')}
        >
          Create artwork
        </button>
        <button
          aria-pressed={view === 'library'}
          disabled={frozen || drawing}
          onClick={() => setView('library')}
        >
          Saved library
        </button>
        <button disabled={frozen || catalog.reading} onClick={() => refresh()}>
          Refresh art library
        </button>
      </nav>
      <p role="status" className="pixel-library-status">
        {catalog.reading ? 'Loading library…' : null}
      </p>
      {catalog.readError && <p role="alert">{catalog.readError}</p>}
      {placement && <p role="status">{placement}</p>}
      {!canAdd && <p>Start Edit layout to place artwork.</p>}
      <div className="pixel-workshop-grid" hidden={view !== 'create'}>
        <section aria-label="Create pixel artwork">
          <fieldset disabled={frozen || drawing}>
            <legend>Drawing tools</legend>
            <div className="pixel-toolbar">
              <button
                disabled={history.cursor === 0}
                onClick={() => {
                  setHistory(pixelHistory.undo);
                  catalog.edited();
                }}
              >
                Undo pixels
              </button>
              <button
                disabled={history.cursor + 1 === history.entries.length}
                onClick={() => {
                  setHistory(pixelHistory.redo);
                  catalog.edited();
                }}
              >
                Redo pixels
              </button>
              <button onClick={() => setConfirmNew(!confirmNew)}>New drawing</button>
            </div>
            {confirmNew && (
              <div className="pixel-new">
                <label>
                  New canvas size
                  <select
                    value={newSize}
                    onChange={(event) => setNewSize(Number(event.target.value) as PixelSize)}
                  >
                    {PIXEL_SIZES.map((size) => (
                      <option value={size} key={size}>
                        {size} × {size} pixels
                      </option>
                    ))}
                  </select>
                </label>
                <p>Replace this drawing and its Undo history. Saved library artwork is kept.</p>
                <button
                  onClick={() => {
                    setHistory(pixelHistory.create(newPixelDraft(newSize)));
                    setConfirmNew(false);
                    catalog.edited();
                    setPlacement(undefined);
                  }}
                >
                  Replace drawing
                </button>
              </div>
            )}
            <div className="pixel-palette" role="group" aria-label="Pixel palette">
              {draft.palette.map((entry, index) => (
                <button
                  key={index}
                  type="button"
                  title={index === 0 ? 'Eraser' : entry}
                  aria-label={index === 0 ? 'Eraser' : `Color ${index}`}
                  aria-pressed={color === index}
                  style={{ backgroundColor: entry }}
                  onClick={() => setColor(index)}
                >
                  {index === 0 ? '×' : ''}
                </button>
              ))}
            </div>
            {color !== 0 && (
              <label>
                Selected color
                <input
                  type="color"
                  value={draft.palette[color]!.slice(0, 7)}
                  onChange={(event) =>
                    change({
                      ...draft,
                      palette: draft.palette.map((entry, index) =>
                        index === color ? `${event.target.value}ff` : entry
                      ),
                    })
                  }
                />
              </label>
            )}
          </fieldset>
          <PixelCanvas
            key={draft.size}
            draft={draft}
            color={color}
            disabled={frozen}
            commit={change}
            strokeChanged={setDrawing}
          />
        </section>
        <section aria-label="Artwork preview">
          <h3>Preview</h3>
          {preview ? <ArtPreview pack={preview} /> : <p>{invalid}</p>}
          <p>
            Flat artwork · same image from every direction · {draft.size / 8} × {draft.size / 8}{' '}
            tiles
          </p>
          <fieldset disabled={frozen || drawing}>
            <legend>Artwork details</legend>
            <label>
              Artwork name
              <input
                maxLength={80}
                value={draft.label}
                onChange={(event) => change({ ...draft, label: event.target.value })}
              />
            </label>
            <label>
              Credit
              <input
                maxLength={120}
                value={draft.credit}
                onChange={(event) => change({ ...draft, credit: event.target.value })}
              />
            </label>
            <label>
              License
              <input
                maxLength={64}
                value={draft.license}
                onChange={(event) => change({ ...draft, license: event.target.value })}
              />
            </label>
            <p>Private by default. Saving does not publish or grant a public license.</p>
          </fieldset>
          <button
            disabled={
              frozen ||
              drawing ||
              !preview ||
              !catalog.page ||
              catalog.reading ||
              catalog.save.kind === 'conflict' ||
              catalog.save.kind === 'saved'
            }
            onClick={() => void catalog.install(preview)}
          >
            Save artwork to library
          </button>
          {catalog.save.kind === 'pending' && (
            <p role="status">Saving artwork… Do not reload this page.</p>
          )}
          {catalog.save.kind === 'uncertain' && (
            <div role="alert">
              <p>
                Save was not confirmed. The exact drawing and revision are frozen; it may already be
                stored.
              </p>
              <button onClick={() => void catalog.install()}>Retry exact save</button>
              <button onClick={catalog.release}>Release frozen save</button>
              <p>
                Releasing does not remove anything already stored. Refresh the library before saving
                again.
              </p>
            </div>
          )}
          {(catalog.save.kind === 'conflict' || catalog.save.kind === 'rejected') && (
            <p role="alert">{catalog.save.message}</p>
          )}
          {catalog.save.kind === 'saved' && (
            <div>
              <p role="status">
                Artwork saved. It stays in your library even if you cancel the layout.
              </p>
              <button
                disabled={!canAdd}
                onClick={() => {
                  if (catalog.save.kind === 'saved') place(catalog.save.catalog, 'artwork');
                }}
              >
                Add saved artwork to layout draft
              </button>
            </div>
          )}
        </section>
      </div>
      <section aria-label="Saved art library" hidden={view !== 'library'}>
        <h3>Saved art library</h3>
        {!catalog.page && !catalog.reading && <p>Refresh the library to see saved artwork.</p>}
        {catalog.page && (
          <>
            {catalog.page.entries.length === 0 && <p>No custom artwork on this page.</p>}
            <div className="pixel-library-list">
              {catalog.page.entries.map((entry) => (
                <button
                  key={entry.digest}
                  disabled={catalog.reading}
                  aria-pressed={selected === entry.digest}
                  onClick={() => setSelected(entry.digest)}
                >
                  {entry.label}
                </button>
              ))}
            </div>
            {catalog.page.excluded.length > 0 && (
              <p role="alert">
                {catalog.page.excluded.length} invalid artwork packs were excluded.
              </p>
            )}
            {catalog.page.nextCursor && (
              <button
                disabled={frozen || catalog.reading}
                onClick={() => refresh(catalog.page!.nextCursor!)}
              >
                Next library page
              </button>
            )}
          </>
        )}
        {selected && (
          <LibraryArt key={selected} digest={selected} port={port} canAdd={canAdd} add={place} />
        )}
      </section>
    </section>
  );
}
