import { useEffect, useState } from 'react';
import type { MeetingRoom, RoomPort } from './room-contract.js';
import { RoomEditor } from './room-editor.js';
import { DISPATCH_RECIPIENT_LIMIT } from './dispatch-contract.js';
import type { IdentityChoice } from '../profiles/identity-choice.js';
import type { Presence } from '../identities/presence.js';
import './room-picker.css';

interface SharedProps {
  port: RoomPort;
  choices: (IdentityChoice & { presence: Presence })[];
  onDraftChange?(dirty: boolean): void;
  onSaved?(room: MeetingRoom): void;
  roomId?: string;
  addMember?: IdentityChoice;
}
type Props = SharedProps &
  (
    | { purpose?: 'audience'; onUse(room?: MeetingRoom): void }
    | { purpose: 'manage'; onUse?: never }
  );

/** Selection previews a real stored roster; editing a room never dispatches work. */
export function RoomPicker({
  port,
  choices,
  onUse,
  onDraftChange,
  onSaved,
  purpose = 'audience',
  roomId,
  addMember,
}: Props) {
  const [rooms, setRooms] = useState<MeetingRoom[]>();
  const [selected, select] = useState(roomId ?? '');
  const [reload, setReload] = useState(0);
  const [error, setError] = useState(false);
  const [editing, setEditing] = useState<{
    room?: MeetingRoom;
    retire?: true;
    initialMember?: IdentityChoice;
  }>();
  useEffect(() => {
    onDraftChange?.(Boolean(editing));
  }, [editing, onDraftChange]);
  useEffect(() => {
    const lifetime = new AbortController();
    setRooms(undefined);
    setError(false);
    void port.list(lifetime.signal).then(
      (items) => {
        if (!lifetime.signal.aborted) setRooms(items);
      },
      () => {
        if (!lifetime.signal.aborted) setError(true);
      }
    );
    return () => lifetime.abort();
  }, [port, reload]);
  const current = rooms?.find((room) => room.id === selected);
  const names = new Map(choices.map((item) => [item.id, item.name]));
  return (
    <section className="meeting-room-picker" aria-label="Meeting room selection">
      {addMember && <p>Choose a room for {addMember.name}. Review its members before saving.</p>}
      <label>
        Meeting room
        <select
          value={selected}
          disabled={Boolean(roomId) || !rooms || Boolean(editing)}
          onChange={(event) => {
            select(event.target.value);
            onUse?.();
          }}
        >
          <option value="">Choose a room</option>
          {rooms?.map((room) => (
            <option key={room.id} value={room.id}>
              {room.name}
            </option>
          ))}
        </select>
      </label>
      {!rooms && !error && <p role="status">Loading rooms…</p>}
      {error && <p role="alert">Could not load meeting rooms. No recipients have changed.</p>}
      {!roomId && rooms?.length === 0 && (
        <p>No meeting rooms yet. Create one and choose its members.</p>
      )}
      {roomId && rooms && !current && (
        <p role="alert">This meeting room is unavailable. No other audience was selected.</p>
      )}
      {current && !editing && (
        <>
          <p>
            Revision {current.revision} · {current.memberIds.length} members
          </p>
          <ul aria-label="Current room roster">
            {current.memberIds.map((id) => (
              <li key={id}>{names.get(id) ?? id}</li>
            ))}
          </ul>
          {purpose === 'audience' && (
            <button
              type="button"
              disabled={!current.memberIds.length || Boolean(editing)}
              onClick={() => onUse?.(current)}
            >
              Use this roster
            </button>
          )}
          {!current.memberIds.length && (
            <p>An empty room can be linked to an area but cannot receive requests.</p>
          )}
        </>
      )}
      {!editing && (
        <div className="whiteboard-review-actions">
          <button type="button" onClick={() => setReload(reload + 1)}>
            Refresh rooms
          </button>
          {!roomId && (
            <button
              type="button"
              disabled={Boolean(editing)}
              onClick={() => setEditing({ initialMember: addMember })}
            >
              Create room
            </button>
          )}
          {current && addMember && (
            <button
              type="button"
              disabled={
                Boolean(editing) ||
                current.memberIds.includes(addMember.id) ||
                current.memberIds.length >= DISPATCH_RECIPIENT_LIMIT
              }
              onClick={() => setEditing({ room: current, initialMember: addMember })}
            >
              {current.memberIds.includes(addMember.id)
                ? `${addMember.name} is already a member`
                : `Add ${addMember.name}`}
            </button>
          )}
          {current &&
            addMember &&
            !current.memberIds.includes(addMember.id) &&
            current.memberIds.length >= DISPATCH_RECIPIENT_LIMIT && (
              <p>This room is full. Edit its members before adding another identity.</p>
            )}
          {current && (!roomId || purpose === 'manage') && (
            <button
              type="button"
              disabled={Boolean(editing)}
              onClick={() => setEditing({ room: current })}
            >
              Edit room
            </button>
          )}
          {current && purpose === 'manage' && (
            <button
              type="button"
              disabled={Boolean(editing)}
              onClick={() => setEditing({ room: current, retire: true })}
            >
              Retire room
            </button>
          )}
        </div>
      )}
      {editing && (
        <RoomEditor
          key={editing.room?.id ?? 'new'}
          port={port}
          choices={choices}
          room={editing.room}
          initialMember={editing.initialMember}
          retiring={editing.retire ?? false}
          close={() => setEditing(undefined)}
          saved={(room) => {
            // The list has one read owner. Invalidate any pre-save read instead of
            // letting its late response overwrite a separately merged save result.
            setReload((value) => value + 1);
            select(room.retired ? '' : room.id);
            setEditing(undefined);
            onUse?.();
            onSaved?.(room);
          }}
        />
      )}
    </section>
  );
}
