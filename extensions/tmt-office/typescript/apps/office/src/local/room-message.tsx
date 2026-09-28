import { useCallback, useContext, useEffect, useRef, useState, useSyncExternalStore } from 'react';
import { useExtensionPanel } from '../extensions/use-extension-panel.js';
import { DispatchComposer } from './dispatch-composer.js';
import { createDispatchComposerState } from './dispatch-composer-state.js';
import type { DispatchComposerState } from './dispatch-composer-state.js';
import { createDispatchJournal } from './dispatch-journal.js';
import { LocalRuntimeContext } from './local-runtime.js';
import type { LocalRuntime } from './local-runtime.js';
import type { MeetingRoom } from './room-contract.js';

type Destination = Pick<MeetingRoom, 'id' | 'name'>;
type LeaveState = 'ready' | 'draft' | 'pending';

/** One retained room composition; an unresolved operation cannot silently change rooms. */
export function useRoomMessage() {
  const [target, select] = useState<Destination>();
  const [requested, request] = useState<Destination>();
  const [leave, setLeave] = useState<LeaveState>('ready');
  const currentLeave = useRef<LeaveState>('ready');
  const changed = useCallback((value: LeaveState) => {
    currentLeave.current = value;
    setLeave(value);
  }, []);
  const choose = useCallback((value: Destination) => {
    select(value);
    request(undefined);
    currentLeave.current = 'ready';
    setLeave('ready');
  }, []);
  const modal = useExtensionPanel(
    'Message room',
    (active) => (
      <>
        {requested && (
          <section aria-label="Switch room message confirmation">
            <h2>Message {requested.name} instead?</h2>
            <p>
              {leave === 'pending'
                ? 'Resolve or explicitly discard the current pending request before switching. It may already be queued.'
                : 'Switching discards this unsent room draft. Accepted requests and replies are not deleted.'}
            </p>
            <button
              disabled={leave === 'pending'}
              onClick={() => {
                if (currentLeave.current !== 'pending') choose(requested);
              }}
            >
              Discard draft and switch room
            </button>
            <button onClick={() => request(undefined)}>Keep this room message</button>
          </section>
        )}
        {target && (
          <RoomMessage key={target.id} target={target} active={active} changed={changed} />
        )}
      </>
    ),
    'room-message-panel'
  );
  return {
    panel: modal.panel,
    open(room: Destination) {
      if (target?.id === room.id) request(undefined);
      else if (target && currentLeave.current !== 'ready') request(room);
      else choose(room);
      modal.open();
    },
  };
}

function RoomMessage({
  target,
  active,
  changed,
}: {
  target: Destination;
  active: boolean;
  changed(value: LeaveState): void;
}) {
  const runtime = useContext(LocalRuntimeContext);
  const [state, setState] = useState<DispatchComposerState>();
  useEffect(() => {
    if (!runtime) return;
    const owner = createDispatchComposerState(
      runtime.dispatch,
      { kind: 'request', message: (text) => text },
      undefined,
      {
        journal: createDispatchJournal({ kind: 'roster', roomId: target.id }),
        lookup: (id, signal) => runtime.requests.receipt(id, signal),
      }
    );
    setState(owner);
    return () => owner.dispose();
  }, [runtime, target.id]);
  useEffect(() => {
    if (active && state) void state.recover();
  }, [active, state]);
  if (!runtime) return <p role="alert">Start a local Office session to message a room.</p>;
  return (
    <>
      <h2>Message {target.name}</h2>
      <p>
        Request a reply from every member of this meeting room. Review the exact audience before
        sending.
      </p>
      {state ? (
        <RoomMessageSession state={state} runtime={runtime} roomId={target.id} changed={changed} />
      ) : (
        <p role="status">Opening room message…</p>
      )}
    </>
  );
}

function RoomMessageSession({
  state,
  runtime,
  roomId,
  changed,
}: {
  state: DispatchComposerState;
  runtime: LocalRuntime;
  roomId: string;
  changed(value: LeaveState): void;
}) {
  const snapshot = useSyncExternalStore(state.subscribe, state.getSnapshot);
  const leave: LeaveState =
    snapshot.busy || (!snapshot.receipt && (snapshot.attempted || snapshot.recoveryBlocked))
      ? 'pending'
      : !snapshot.receipt && (snapshot.text || snapshot.recipients.length)
        ? 'draft'
        : 'ready';
  useEffect(() => changed(leave), [changed, leave]);
  return (
    <DispatchComposer
      state={state}
      profiles={runtime.profiles}
      rooms={runtime.rooms}
      roomId={roomId}
      copy={{
        label: 'Compose a room request',
        heading: 'Room request',
        inputLabel: 'Message',
        placeholder: 'What should this room work on?',
        audienceHint: 'Only these room members will receive this request:',
      }}
    />
  );
}
