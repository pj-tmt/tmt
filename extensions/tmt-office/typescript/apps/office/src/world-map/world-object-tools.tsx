import type { WorldObject } from './world-contract.js';
import { useEffect, useState } from 'react';
import { externalLink } from '../extensions/external-link.js';
import { WorldObjectCoordinates } from './world-object-coordinates.js';
import { NotebookAttachment } from '../notebooks/notebook-attachment.js';
import type { ProfileProjection } from '../profiles/profile-contract.js';
import { rotatedObject } from './object-rotation.js';

/** Edits one placement inside the parent's whole-world draft, never its resource. */
export function WorldObjectTools({
  object,
  change,
  remove,
  identities,
  mountOnWall,
  wallEditing = true,
  canRotate = true,
  rotate,
  rotationProblem,
}: {
  object: WorldObject;
  change: (object: WorldObject) => void;
  remove: () => void;
  identities: ProfileProjection[];
  mountOnWall?: () => void;
  wallEditing?: boolean;
  canRotate?: boolean;
  rotate?: () => void;
  rotationProblem?: string;
}) {
  const { surface } = object;
  const existingUrl =
    object.extension?.binding.kind === 'external-link' ? object.extension.binding.url : '';
  const [url, setUrl] = useState(existingUrl);
  const [linkError, setLinkError] = useState<string>();
  useEffect(() => {
    setUrl(existingUrl);
    setLinkError(undefined);
  }, [object.id, existingUrl]);
  return (
    <section aria-label="Selected object">
      <div className="world-object-actions">
        <button onClick={remove}>Delete</button>
      </div>
      <p>
        {canRotate
          ? 'Drag a corner handle to rotate. Delete removes the selected object.'
          : 'This artwork has one view; directional rotation is unavailable.'}
      </p>
      <details className="world-placement-details">
        <summary>Precise placement</summary>
        <button
          disabled={!canRotate || Boolean(rotationProblem)}
          title={rotationProblem ?? (!canRotate ? 'Directional artwork is unavailable' : undefined)}
          onClick={() => (rotate ? rotate() : change(rotatedObject(object, 1)))}
        >
          Rotate 90° clockwise
        </button>
        {rotationProblem && <p role="status">{rotationProblem}</p>}
        {mountOnWall && (
          <button onClick={mountOnWall}>Place on a suitable wall in this area</button>
        )}
        <p>{object.id}</p>
        {wallEditing && (
          <>
            <label>
              Object kind
              <select
                value={object.kind}
                onChange={(event) => {
                  const kind = event.target.value as WorldObject['kind'];
                  change({
                    ...object,
                    kind,
                    surface:
                      kind !== 'decoration' && surface.type === 'floor'
                        ? { type: 'wall', axis: 'horizontal', face: 'positive', elevation: 0 }
                        : surface,
                  });
                }}
              >
                <option value="decoration">Decoration</option>
                <option value="window">Window · exterior wall only</option>
                <option value="wallLight">Wall light · static glow</option>
              </select>
            </label>
            <p>
              Changing kind or surface keeps the coordinates. Use the wall placement action to find
              support in this area.
            </p>
            <label>
              Placement surface
              <select
                value={surface.type}
                onChange={(event) =>
                  change({
                    ...object,
                    surface:
                      event.target.value === 'floor'
                        ? { type: 'floor' }
                        : {
                            type: 'wall',
                            axis: 'horizontal',
                            face: 'positive',
                            elevation: 0,
                          },
                  })
                }
              >
                <option value="floor" disabled={object.kind !== 'decoration'}>
                  Floor
                </option>
                <option value="wall">Wall</option>
              </select>
            </label>
            {surface.type === 'wall' && (
              <>
                <label>
                  Wall direction
                  <select
                    value={surface.axis}
                    onChange={(event) =>
                      change({
                        ...object,
                        surface: {
                          ...surface,
                          axis: event.target.value as typeof surface.axis,
                        },
                      })
                    }
                  >
                    <option value="horizontal">Horizontal · extends east</option>
                    <option value="vertical">Vertical · extends south</option>
                  </select>
                </label>
                <label>
                  Indoor face
                  <select
                    value={surface.face}
                    onChange={(event) =>
                      change({
                        ...object,
                        surface: {
                          ...surface,
                          face: event.target.value as typeof surface.face,
                        },
                      })
                    }
                  >
                    <option value="positive">
                      {surface.axis === 'horizontal' ? 'South' : 'East'}
                    </option>
                    <option value="negative">
                      {surface.axis === 'horizontal' ? 'North' : 'West'}
                    </option>
                  </select>
                </label>
                <p>
                  X and Y locate the starting wall edge. Elevation is above the floor. Save checks
                  continuous wall support, indoor face and door clearance.
                </p>
              </>
            )}
          </>
        )}
        <WorldObjectCoordinates
          key={JSON.stringify([object.id, object.placement, surface])}
          object={object}
          change={change}
        />
      </details>
      {object.extension && <p>Removing this placement keeps its linked content.</p>}
      <details key={object.id} className="world-object-binding">
        <summary>Object action</summary>
        <NotebookAttachment
          key={JSON.stringify([object.id, object.extension])}
          object={object}
          identities={identities}
          change={change}
        />
        {object.extension && object.extension.binding.kind !== 'external-link' ? (
          object.extension.binding.kind !== 'notebook' && (
            <p>This object opens an existing resource. Its binding is retained.</p>
          )
        ) : (
          <form
            aria-label="Web link"
            onSubmit={(event) => {
              event.preventDefault();
              try {
                externalLink(url);
                change({
                  ...object,
                  extension: { definition: 'tmt-link', binding: { kind: 'external-link', url } },
                });
                setLinkError(undefined);
              } catch (cause) {
                setLinkError(cause instanceof Error ? cause.message : 'Invalid web destination.');
              }
            }}
          >
            <label>
              Web destination
              <input
                type="url"
                required
                value={url}
                onChange={(event) => setUrl(event.target.value)}
                placeholder="https://example.com/docs"
              />
            </label>
            <button type="submit">{existingUrl ? 'Update link' : 'Attach link to object'}</button>
            {existingUrl && (
              <button type="button" onClick={() => change({ ...object, extension: null })}>
                Remove link action
              </button>
            )}
            {linkError && <p role="alert">{linkError}</p>}
            <p>
              Attaching a link does not open it. Visitors review the destination before opening a
              website.
            </p>
          </form>
        )}
      </details>
    </section>
  );
}
