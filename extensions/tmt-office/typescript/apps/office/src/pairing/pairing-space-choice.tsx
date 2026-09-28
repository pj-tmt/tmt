import { useContext, useEffect, useState, useSyncExternalStore } from 'react';
import { SpaceContext } from '../spaces/space-view.js';
import { createSpaceState } from '../spaces/space-state.js';
import type { SpaceState } from '../spaces/space-state.js';
import type { PairingState } from './pairing-state.js';
import type { PairingRequest } from './pairing-contract.js';

/** Selection is approval intent; pages remain owned by the existing space state. */
export function PairingSpaceChoice({
  state,
  request,
  ownerUid,
}: {
  state: PairingState;
  request: PairingRequest;
  ownerUid: string;
}) {
  const port = useContext(SpaceContext);
  const [spaces, setSpaces] = useState<SpaceState>();
  useEffect(() => {
    if (!port) return;
    const next = createSpaceState(port, request.worldId, ownerUid);
    setSpaces(next);
    void next.refresh();
    return () => next.dispose();
  }, [port, request.worldId, ownerUid]);
  return spaces ? <SpaceChoice state={state} spaces={spaces} /> : null;
}

function SpaceChoice({ state, spaces }: { state: PairingState; spaces: SpaceState }) {
  const { attempted, revocationAttempted, replacement } = useSyncExternalStore(
    state.subscribe,
    state.getSnapshot
  );
  const locked = attempted || revocationAttempted;
  const { entries, busy, next, error, ready } = useSyncExternalStore(
    spaces.subscribe,
    spaces.getSnapshot
  );
  return (
    <fieldset className="pairing-space-choice" disabled={locked}>
      <legend>Assigned block</legend>
      <p>
        Choose a fresh block or a revoked agent's retained layout. Profiles and notebooks are not
        transferred.
      </p>
      <label>
        <input
          type="radio"
          name="pairing-block"
          checked={!replacement}
          onChange={() => state.selectReplacement(null)}
        />{' '}
        New empty block
      </label>
      {replacement && (
        <p>
          Selected retained block: <code>{replacement.blockId}</code> · Source grant:{' '}
          <code>{replacement.principalUid}</code>
        </p>
      )}
      {locked ? (
        <p>The choice is fixed for this request. Retry keeps the same assignment.</p>
      ) : (
        <>
          <button type="button" disabled={busy} onClick={() => void spaces.refresh()}>
            Refresh retained spaces
          </button>
          <button type="button" disabled={busy || !next} onClick={() => void spaces.next()}>
            Next retained spaces
          </button>
          {busy && <p role="status">Loading retained spaces…</p>}
          {error && <p role="alert">{error}</p>}
          {ready && !entries.some((entry) => !entry.enabled) && (
            <p>No revoked grants on this page.</p>
          )}
          {entries
            .filter((entry) => !entry.enabled)
            .map((entry) => (
              <label key={entry.principalUid}>
                <input
                  type="radio"
                  name="pairing-block"
                  checked={replacement?.principalUid === entry.principalUid}
                  onChange={() =>
                    state.selectReplacement({
                      principalUid: entry.principalUid,
                      blockId: entry.blockId,
                    })
                  }
                />
                Retained block <code>{entry.blockId}</code> · Previous identity{' '}
                <code>{entry.identityId}</code>
                {entry.replacedByPairingId && (
                  <span>
                    {' '}
                    — Reserved or transferred. Approval succeeds only if the earlier reservation is
                    unclaimed and expired or revoked.
                  </span>
                )}
              </label>
            ))}
        </>
      )}
    </fieldset>
  );
}
