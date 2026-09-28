import { useEffect, useState, useSyncExternalStore } from 'react';
import { DISPATCH_RECIPIENT_LIMIT } from './dispatch-contract.js';
import type { ProfilePort, ProfileProjection } from '../profiles/profile-contract.js';
import type { DispatchComposerState } from './dispatch-composer-state.js';
import type { RoomPort } from './room-contract.js';
import { RoomPicker } from './room-picker.js';
import { IdentityChecklist } from '../profiles/identity-checklist.js';
import './dispatch-composer.css';

interface Props {
  state: DispatchComposerState;
  profiles: ProfilePort;
  rooms: RoomPort;
  onDraftChange?(dirty: boolean): void;
  roomId?: string;
  copy: {
    label: string;
    heading: string;
    inputLabel: string;
    placeholder: string;
    audienceHint: string;
  };
}

export function DispatchComposer({ state, profiles, rooms, onDraftChange, roomId, copy }: Props) {
  const noun = state.kind;
  const snapshot = useSyncExternalStore(state.subscribe, state.getSnapshot);
  const [directory, setDirectory] = useState<ProfileProjection[]>();
  const [directoryError, setDirectoryError] = useState(false);
  const [reload, setReload] = useState(0);
  const [confirmDiscard, setConfirmDiscard] = useState(false);
  const [mode, setMode] = useState<'identities' | 'room'>(roomId ? 'room' : 'identities');
  const [roomDraft, setRoomDraft] = useState(false);
  const dirty =
    roomDraft ||
    (!snapshot.receipt &&
      Boolean(snapshot.text || snapshot.recipients.length || snapshot.recoveryBlocked));
  useEffect(() => {
    let active = true;
    setDirectory(undefined);
    setDirectoryError(false);
    const controller = new AbortController();
    void profiles.list(controller.signal).then(
      (items) => {
        if (active) setDirectory(items);
      },
      () => {
        if (active) setDirectoryError(true);
      }
    );
    return () => {
      active = false;
      controller.abort();
    };
  }, [profiles, reload]);
  useEffect(() => {
    onDraftChange?.(dirty);
    if (!dirty) return;
    const warn = (event: BeforeUnloadEvent) => {
      event.preventDefault();
    };
    window.addEventListener('beforeunload', warn);
    return () => window.removeEventListener('beforeunload', warn);
  }, [dirty, onDraftChange]);

  const names = new Map(snapshot.recipients.map((item) => [item.id, item.name]));
  const choices =
    directory?.map((profile) => ({
      id: profile.identityId,
      name: profile.identityName,
      presence: profile.presence,
    })) ?? [];
  return (
    <section className="dispatch-composer" aria-label={copy.label}>
      <h3>{copy.heading}</h3>
      {snapshot.receipt ? (
        <>
          <p role="status">
            {noun === 'request' ? 'Request' : 'Announcement'} recorded. Queued means saved to the
            inbox, not read or completed.
          </p>
          {snapshot.error && <p role="alert">{snapshot.error}</p>}
          <ul className="dispatch-receipts">
            {snapshot.receipt.items.map((item) => (
              <li key={item.recipientId}>
                <strong>{names.get(item.recipientId)}</strong> ·{' '}
                {item.acceptance === 'queued' ? 'Queued' : 'Unavailable · not queued'}
                <code>{item.requestId}</code>
              </li>
            ))}
          </ul>
          {noun === 'request' ? (
            <p>
              Read replies with <code>tmt result &lt;request-id&gt;</code>.
            </p>
          ) : (
            <p>No reply is requested. Recipients acknowledge after reading.</p>
          )}
          <button
            onClick={() => {
              state.discard();
              setConfirmDiscard(false);
            }}
          >
            Compose another {noun}
          </button>
        </>
      ) : snapshot.review ? (
        <>
          <p>
            {snapshot.room
              ? `${snapshot.room.name} · revision ${snapshot.room.revision}`
              : snapshot.review.room?.kind === 'roster'
                ? `Room ${snapshot.review.room.roomId} · revision ${snapshot.review.room.revision}`
                : 'Selected agents'}
          </p>
          <p>{copy.audienceHint}</p>
          <ul aria-label="Confirmed recipients">
            {snapshot.recipients.map((item) => (
              <li key={item.id}>{item.name}</li>
            ))}
          </ul>
          <label>
            Exact {noun}
            <textarea readOnly value={snapshot.review.message} rows={7} />
          </label>
          {snapshot.error && <p role="alert">{snapshot.error}</p>}
          <div className="dispatch-actions">
            {!snapshot.rejected && (
              <button
                className="dispatch-primary"
                disabled={snapshot.busy}
                onClick={() => void state.send()}
              >
                {snapshot.busy ? 'Sending…' : snapshot.attempted ? 'Retry send' : `Send ${noun}`}
              </button>
            )}
            {!snapshot.attempted && <button onClick={() => state.edit()}>Edit {noun}</button>}
          </div>
        </>
      ) : (
        <div>
          <label>
            {copy.inputLabel}
            <textarea
              required
              value={snapshot.text}
              placeholder={copy.placeholder}
              onChange={(event) => state.change(event.target.value, snapshot.recipients)}
            />
          </label>
          {!roomId && (
            <fieldset className="dispatch-selection" disabled={roomDraft}>
              <legend>Send to</legend>
              {(['identities', 'room'] as const).map((value) => (
                <label key={value}>
                  <input
                    type="radio"
                    checked={mode === value}
                    onChange={() => {
                      setMode(value);
                      state.chooseRoom(undefined, []);
                    }}
                  />
                  {value === 'room' ? 'Meeting room' : 'Selected agents'}
                </label>
              ))}
            </fieldset>
          )}
          {mode === 'identities' ? (
            <IdentityChecklist
              legend="Recipients"
              choices={choices}
              selected={snapshot.recipients}
              limit={DISPATCH_RECIPIENT_LIMIT}
              onChange={(recipients) => state.change(snapshot.text, recipients)}
            />
          ) : (
            <RoomPicker
              port={rooms}
              roomId={roomId}
              choices={choices}
              onDraftChange={setRoomDraft}
              onUse={(room) =>
                state.chooseRoom(
                  room,
                  room?.memberIds.map((id) => ({
                    id,
                    name: choices.find((item) => item.id === id)?.name ?? id,
                  })) ?? []
                )
              }
            />
          )}
          {snapshot.room && (
            <p>
              {noun === 'request' ? 'Request' : 'Announcement'} roster: {snapshot.room.name} ·
              revision {snapshot.room.revision} · {snapshot.recipients.length} agents
            </p>
          )}
          {!directory && !directoryError && <p role="status">Loading agents…</p>}
          {directory?.length === 0 && <p>No agents yet. Create an identity with tmt first.</p>}
          {directoryError && <p role="alert">Could not load agents. Your draft is kept.</p>}
          {directoryError && (
            <button type="button" onClick={() => setReload(reload + 1)}>
              Retry loading agents
            </button>
          )}
          {directory && (
            <button type="button" onClick={() => setReload(reload + 1)}>
              Refresh agents
            </button>
          )}
          {snapshot.error && <p role="alert">{snapshot.error}</p>}
          <button
            type="button"
            onClick={() => state.review()}
            disabled={
              roomDraft ||
              snapshot.recoveryBlocked ||
              snapshot.roomStale ||
              !snapshot.text.trim() ||
              !snapshot.recipients.length
            }
          >
            Review {noun}
          </button>
          <p className="dispatch-tip">
            Offline agents can collect this {noun} from their inbox later.
          </p>
        </div>
      )}
      {dirty &&
        !snapshot.busy &&
        !roomDraft &&
        (confirmDiscard ? (
          <div
            className="dispatch-discard"
            role="group"
            aria-label={`Discard ${noun} confirmation`}
          >
            <p>
              {snapshot.attempted
                ? `This does not cancel any queued work. A new ${noun} could duplicate it. Discard anyway?`
                : `Discard this unsent ${noun}?`}
            </p>
            <button
              onClick={() => {
                state.discard();
                setConfirmDiscard(false);
              }}
            >
              Confirm discard
            </button>
            <button onClick={() => setConfirmDiscard(false)}>Keep {noun}</button>
          </div>
        ) : (
          <button onClick={() => setConfirmDiscard(true)}>Discard {noun}</button>
        ))}
    </section>
  );
}
