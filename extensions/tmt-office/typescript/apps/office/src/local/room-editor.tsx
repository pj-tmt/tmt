import { useEffect, useRef, useState } from 'react';
import { DISPATCH_RECIPIENT_LIMIT } from './dispatch-contract.js';
import type { MeetingRoom, RoomPort } from './room-contract.js';
import { IdentityChecklist } from '../profiles/identity-checklist.js';
import type { IdentityChoice } from '../profiles/identity-choice.js';
import type { Presence } from '../identities/presence.js';

export function RoomEditor({
  port,
  choices,
  room,
  retiring,
  close,
  saved,
  showMembers = true,
  onBusyChange,
  initialMember,
}: {
  port: RoomPort;
  choices: (IdentityChoice & { presence: Presence })[];
  showMembers?: boolean;
  onBusyChange?(busy: boolean): void;
  room?: MeetingRoom;
  /** An explicit agent entry point seeds the draft, never the stored roster. */
  initialMember?: IdentityChoice;
  retiring: boolean;
  close(): void;
  saved(room: MeetingRoom): void;
}) {
  const [id] = useState(() => room?.id ?? crypto.randomUUID());
  const [name, setName] = useState(room?.name ?? '');
  const [members, setMembers] = useState<IdentityChoice[]>(() => {
    const existing =
      room?.memberIds.map((id) => ({
        id,
        name: choices.find((item) => item.id === id)?.name ?? id,
      })) ?? [];
    if (initialMember && !existing.some((member) => member.id === initialMember.id))
      existing.push(initialMember);
    return existing;
  });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [observed, setObserved] = useState<MeetingRoom>();
  const lifetime = useRef<AbortController>(undefined);
  const savedCallback = useRef(saved);
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => {
    savedCallback.current = saved;
  }, [saved]);
  useEffect(() => {
    onBusyChange?.(busy);
    return () => onBusyChange?.(false);
  }, [busy, onBusyChange]);
  useEffect(() => {
    input.current?.focus({ preventScroll: true });
  }, []);
  useEffect(() => {
    const owner = new AbortController();
    lifetime.current = owner;
    return () => owner.abort();
  }, [port]);
  return (
    <form
      aria-label="Meeting room editor"
      onSubmit={(event) => {
        event.preventDefault();
        const signal = lifetime.current?.signal;
        if (busy || !signal || signal.aborted) return;
        setBusy(true);
        setError(undefined);
        setObserved(undefined);
        const mutation =
          retiring && room
            ? port.retire(id, room.revision, signal)
            : port.save(
                id,
                {
                  expectedRevision: room?.revision ?? 0,
                  name,
                  memberIds: members.map((item) => item.id),
                },
                signal
              );
        void mutation.then(
          (result) => {
            if (!signal.aborted) savedCallback.current(result);
          },
          () => {
            if (!signal.aborted) {
              setBusy(false);
              setError(
                retiring
                  ? 'Retirement was not confirmed or the room changed. It may already have completed. Retry uses the same room and revision; closing this review does not undo retirement.'
                  : 'Save was not confirmed or the room changed. Your draft is kept. Check the saved room before retrying or replacing this draft.'
              );
            }
          }
        );
      }}
    >
      <h4>{retiring ? `Retire ${room?.name}` : room ? 'Edit meeting room' : 'New meeting room'}</h4>
      {retiring ? (
        <p>
          Stop new requests and room selection. Keep the area, furniture, members, discussions,
          whiteboards and delivered requests. This cannot be undone.
        </p>
      ) : (
        <>
          <label>
            Room name
            <input
              ref={input}
              type="text"
              required
              disabled={busy}
              value={name}
              onChange={(event) => setName(event.target.value)}
            />
          </label>
          {showMembers && (
            <IdentityChecklist
              legend="Room members"
              choices={choices}
              selected={members}
              limit={DISPATCH_RECIPIENT_LIMIT}
              disabled={busy}
              onChange={setMembers}
            />
          )}
        </>
      )}
      {error && <p role="alert">{error}</p>}
      {error && !retiring && (
        <button
          type="button"
          disabled={busy}
          onClick={() => {
            const signal = lifetime.current?.signal;
            if (busy || !signal || signal.aborted) return;
            setBusy(true);
            setObserved(undefined);
            void port.list(signal).then(
              (rooms) => {
                if (signal.aborted) return;
                setBusy(false);
                const current = rooms.find((item) => item.id === id);
                setObserved(current);
                if (!current)
                  setError(
                    'This room was not found in the active list. Your draft and creation ID are kept; retry does not create another ID.'
                  );
              },
              () => {
                if (signal.aborted) return;
                setBusy(false);
                setError('The saved room could not be checked. Your draft is kept.');
              }
            );
          }}
        >
          Check saved room
        </button>
      )}
      {observed && (
        <section aria-label="Saved room review">
          <p>
            {observed.name} · Revision {observed.revision} · {observed.memberIds.length} members
          </p>
          <ul>
            {observed.memberIds.map((memberId) => (
              <li key={memberId}>
                {choices.find((item) => item.id === memberId)?.name ?? memberId}
              </li>
            ))}
          </ul>
          <p>
            Use this stored room instead of the draft. This does not write or overwrite anything.
          </p>
          <button type="button" disabled={busy} onClick={() => savedCallback.current(observed)}>
            Use saved room
          </button>
        </section>
      )}
      <button type="submit" disabled={busy}>
        {busy ? 'Saving room…' : retiring ? 'Confirm retirement' : 'Save room'}
      </button>
      <button type="button" disabled={busy} onClick={close}>
        {retiring
          ? error
            ? 'Close retirement review'
            : 'Cancel retirement'
          : 'Discard room draft'}
      </button>
    </form>
  );
}
