import { useState } from 'react';
import type { ProfileProjection } from '../profiles/profile-contract.js';
import type { WorldObject } from '../world-map/world-contract.js';
import { NOTEBOOK_EXTENSION } from '../extensions/bundled-extensions.js';

/** Edits a reference in the world draft; never creates, reads or deletes notes. */
export function NotebookAttachment({
  object,
  identities,
  change,
}: {
  object: WorldObject;
  identities: ProfileProjection[];
  change: (object: WorldObject) => void;
}) {
  const binding = object.extension?.binding;
  const currentId = binding?.kind === 'notebook' ? binding.identityId : '';
  const [identityId, select] = useState(currentId);
  const eligible = identities.filter((identity) => identity.lifetime === 'saved');
  const available = eligible.some((identity) => identity.identityId === identityId);
  // Other resource types must be explicitly detached through their own controls.
  if (binding && binding.kind !== 'notebook') return null;
  return (
    <form
      aria-label="Notebook binding"
      onSubmit={(event) => {
        event.preventDefault();
        if (!available) return;
        change({
          ...object,
          extension: {
            definition: NOTEBOOK_EXTENSION.id,
            binding: { kind: 'notebook', identityId },
          },
        });
      }}
    >
      <label>
        Notebook owner
        <select value={identityId} onChange={(event) => select(event.target.value)} required>
          <option value="">Choose a saved agent</option>
          {identityId && !available && (
            <option value={identityId} disabled>
              Unavailable identity · {identityId}
            </option>
          )}
          {eligible.map((identity) => (
            <option key={identity.identityId} value={identity.identityId}>
              {identity.identityName}
            </option>
          ))}
        </select>
      </label>
      <button type="submit" disabled={!available}>
        {currentId ? 'Update notebook in draft' : 'Attach notebook to object'}
      </button>
      {currentId && (
        <button type="button" onClick={() => change({ ...object, extension: null })}>
          Remove notebook action
        </button>
      )}
      <p>
        Saved agents only. This links existing notes; it does not create or publish a notebook.
        Removing the action keeps the source file.
      </p>
    </form>
  );
}
