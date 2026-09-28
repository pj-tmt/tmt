import { createContext, useContext, useEffect, useState, useSyncExternalStore } from 'react';
import { Link, Navigate, useNavigate } from '@tanstack/react-router';
import type { ReactElement, ReactNode } from 'react';
import type { WorldState } from './world-state.js';
import { WORLD_ID_PATTERN } from './world-contract.js';
import type { World } from './world-contract.js';
import { OfficeSpaces } from '../spaces/space-view.js';

export const WorldContext = createContext<WorldState | undefined>(undefined);

export function WorldGate({
  children,
}: {
  children: (state: WorldState) => ReactNode;
}): ReactElement | null {
  const state = useContext(WorldContext);
  return state ? <Admission state={state}>{children}</Admission> : null;
}

function Admission({
  state,
  children,
}: {
  state: WorldState;
  children: (state: WorldState) => ReactNode;
}): ReactElement | null {
  const { admission, error } = useSyncExternalStore(state.subscribe, state.getSnapshot);
  if (admission === 'signed-out') return null;
  if (admission === 'checking') return <p role="status">Checking pilot access…</p>;
  if (admission === 'waiting' || admission === 'error')
    return (
      <section aria-label="Pilot access">
        <h2>Waiting for access</h2>
        <p>Share your UID above with the operator to enable your test account.</p>
        {error && <p role="alert">{error}</p>}
        <button onClick={state.retryAccess}>Check access again</button>
      </section>
    );
  return <>{children(state)}</>;
}

export function CreateWorld({ state }: { state: WorldState }): ReactElement {
  const [name, setName] = useState('');
  const [worldId, setWorldId] = useState('');
  const [createdId, setCreatedId] = useState<string | null>(null);
  const { busy, error, draft } = useSyncExternalStore(state.subscribe, state.getSnapshot);
  const navigate = useNavigate();
  if (createdId) return <Navigate to="/worlds/$worldId" params={{ worldId: createdId }} />;
  return (
    <section aria-label="Create a world">
      <h2>Create your private world</h2>
      <form
        onSubmit={(event) => {
          event.preventDefault();
          void state.create(name).then(setCreatedId);
        }}
      >
        <label htmlFor="world-name">World name</label>
        <input
          id="world-name"
          value={draft?.name ?? name}
          disabled={busy || !!draft}
          required
          onChange={(event) => setName(event.target.value)}
        />
        <button disabled={busy || (!draft && !name.trim())}>
          {busy ? 'Creating…' : draft ? 'Retry creation' : 'Create world'}
        </button>
      </form>
      {error && <p role="alert">{error}</p>}
      <p>Only your approved account can open this world. Invitations are not available yet.</p>
      <form
        onSubmit={(event) => {
          event.preventDefault();
          void navigate({ to: '/worlds/$worldId', params: { worldId } });
        }}
      >
        <label htmlFor="open-world-id">World ID</label>
        <input
          id="open-world-id"
          value={worldId}
          required
          pattern={WORLD_ID_PATTERN}
          onChange={(event) => setWorldId(event.target.value)}
        />
        <button>Open world</button>
      </form>
    </section>
  );
}

export function SelectedWorld({
  state,
  id,
  children,
}: {
  state: WorldState;
  id: string;
  children?: (world: World) => ReactNode;
}): ReactElement {
  const { world, loading, error } = useSyncExternalStore(state.subscribe, state.getSnapshot);
  useEffect(() => {
    state.select(id);
    return () => state.select();
  }, [state, id]);
  // Route transitions cannot briefly display the previous route's private world.
  if (loading || (world && world.id !== id)) return <p role="status">Opening world…</p>;
  if (!world)
    return (
      <section>
        <h1>World unavailable</h1>
        <p>{error ?? 'No world is available at this address.'}</p>
        <Link to="/">Return to Office</Link>
      </section>
    );
  return (
    <section>
      <p className="eyebrow">Private world</p>
      <h1>{world.name}</h1>
      <p>
        World ID: <code>{world.id}</code>
      </p>
      {children ? children(world) : <OfficeSpaces key={world.id} world={world} />}
    </section>
  );
}
