import { createContext, useContext, useEffect, useState, useSyncExternalStore } from 'react';
import { BlockPanel } from '../blocks/block-view.js';
import type { BlockPort } from '../blocks/block-contract.js';
import type { World } from '../worlds/world-contract.js';
import type { SpacePort } from './space-contract.js';
import { createSpaceState } from './space-state.js';
import type { SpaceState } from './space-state.js';
import './spaces.css';

export const SpaceContext = createContext<SpacePort | undefined>(undefined);
export function OfficeSpaces({ world }: { world: World }) {
  const port = useContext(SpaceContext);
  return port ? (
    <ConnectedSpaces key={world.id} world={world} port={port} />
  ) : (
    <BlockPanel blockKey={world.id} />
  );
}
function ConnectedSpaces({ world, port }: { world: World; port: SpacePort }) {
  const [state, setState] = useState<SpaceState>();
  const [selected, setSelected] = useState<{ id: string; port: BlockPort }>();
  useEffect(() => {
    const next = createSpaceState(port, world.id, world.ownerUid);
    setState(next);
    void next.refresh();
    return () => next.dispose();
  }, [port, world.id, world.ownerUid]);
  return (
    <>
      {state && (
        <SpaceList state={state} open={(id) => setSelected({ id, port: port.block(id) })} />
      )}
      {selected && (
        <p className="space-selection">
          Viewing retained block <code>{selected.id}</code>. Switching spaces discards unsaved
          edits.
          <button onClick={() => setSelected(undefined)}>Open home block</button>
        </p>
      )}
      <BlockPanel
        key={selected?.id ?? 'home'}
        blockKey={world.id}
        blockPort={selected?.port}
        label={selected ? `AGENT SPACE / ${selected.id}` : undefined}
      />
    </>
  );
}
export function SpaceList({ state, open }: { state: SpaceState; open: (id: string) => void }) {
  const { entries, ready, busy, next, error, observedAtMs } = useSyncExternalStore(
    state.subscribe,
    state.getSnapshot
  );
  return (
    <section className="agent-spaces" aria-label="Agent spaces">
      <h2>Agent spaces</h2>
      <p>
        Grant records, not online presence. Expired leases may renew; revoked access does not delete
        a block. Refresh for current records. Switching spaces discards unsaved edits.
      </p>
      <button disabled={busy} onClick={() => void state.refresh()}>
        Refresh spaces
      </button>
      <button disabled={busy || !next} onClick={() => void state.next()}>
        Next spaces
      </button>
      {busy && <p role="status">Loading agent spaces…</p>}
      {error && <p role="alert">{error}</p>}
      {ready && entries.length === 0 && <p>No grant records on this page.</p>}
      <ul>
        {entries.map((entry) => (
          <li key={entry.principalUid}>
            <p>
              Identity: <code>{entry.identityId}</code>
            </p>
            <p>
              Installation: <code>{entry.installationId}</code>
            </p>
            <p>
              Principal: <code>{entry.principalUid}</code>
            </p>
            <p>
              {!entry.enabled
                ? 'Revoked'
                : entry.expiresAtMs <= observedAtMs
                  ? 'Enabled · lease expired at last refresh'
                  : 'Enabled at last refresh'}{' '}
              · Lease expires {new Date(entry.expiresAtMs).toISOString()} ·{' '}
              {entry.capabilities.join(', ')}
            </p>
            <button onClick={() => open(entry.blockId)}>Open block {entry.blockId}</button>
            {entry.replacedByPairingId && (
              <p>
                Reserved or transferred through pairing{' '}
                <code>{entry.replacedByPairingId.slice(0, 12)}</code>. This is not an unassigned
                block.
              </p>
            )}
          </li>
        ))}
      </ul>
    </section>
  );
}
