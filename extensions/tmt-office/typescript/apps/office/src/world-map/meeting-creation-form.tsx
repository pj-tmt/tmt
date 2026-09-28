import { useEffect, useRef, useState } from 'react';
import { RoomEditor } from '../local/room-editor.js';
import type { MeetingRoom, RoomPort } from '../local/room-contract.js';
import type { SelectionTarget } from '../rendering/selection-anchor.js';
import { useAnchoredPanel } from './use-anchored-panel.js';
import type { PanelObstacles } from './use-anchored-panel.js';

/** A canonical-room receipt survives placement failure; retries never create another room. */
export function MeetingCreationForm({
  port,
  rooms,
  anchor,
  obstacles,
  busy,
  onBusyChange,
  roomSaved,
  place,
  close,
  embedded = false,
  linking = false,
}: {
  port: RoomPort;
  rooms: readonly MeetingRoom[];
  anchor?: SelectionTarget;
  obstacles?: PanelObstacles;
  busy: boolean;
  onBusyChange(busy: boolean): void;
  roomSaved(room: MeetingRoom): void;
  place(room: MeetingRoom, areaId: string): boolean;
  close(): void;
  embedded?: boolean;
  linking?: boolean;
}) {
  const panel = useAnchoredPanel<HTMLElement>(anchor, obstacles);
  const [areaId] = useState(() => crypto.randomUUID());
  const [created, setCreated] = useState<MeetingRoom>();
  const [existingId, setExistingId] = useState('');
  const latest = useRef({ place, roomSaved });
  useEffect(() => {
    latest.current = { place, roomSaved };
  }, [place, roomSaved]);
  function attach(room: MeetingRoom) {
    setCreated(room);
    latest.current.roomSaved(room);
    latest.current.place(room, areaId);
  }
  return (
    <section
      ref={panel.ref}
      style={embedded ? undefined : panel.style}
      className={embedded ? 'meeting-link-form' : 'office-expansion-card meeting-expansion-card'}
      aria-label="Create meeting space"
      onKeyDown={(event) => {
        if (event.key === 'Escape' && !busy) {
          event.stopPropagation();
          close();
        }
      }}
    >
      {created ? (
        <>
          <h2>{created.name}</h2>
          <p role="status">The room is saved. Its area link has not been applied.</p>
          <p>
            Retry placement or close and place this existing room later. No duplicate room will be
            created.
          </p>
          <button disabled={busy} onClick={() => latest.current.place(created, areaId)}>
            Retry placement
          </button>
          <button disabled={busy} onClick={close}>
            Close
          </button>
        </>
      ) : (
        <>
          <RoomEditor
            port={port}
            choices={[]}
            retiring={false}
            showMembers={false}
            saved={attach}
            close={close}
            onBusyChange={onBusyChange}
          />
          {linking ? (
            <p>
              Only this area's use changes. Position, furniture and existing resource links stay
              unchanged. Undo keeps the canonical room available.
            </p>
          ) : (
            <p>
              The room is saved first, then its furnished space applies automatically. Undoing the
              placement keeps the room and its history available.
            </p>
          )}
          {rooms.length > 0 && (
            <details>
              <summary>Place an existing room</summary>
              <label>
                Existing room
                <select
                  value={existingId}
                  disabled={busy}
                  onChange={(event) => setExistingId(event.target.value)}
                >
                  <option value="">Choose a room</option>
                  {rooms
                    .filter((room) => !room.retired)
                    .map((room) => (
                      <option key={room.id} value={room.id}>
                        {room.name}
                      </option>
                    ))}
                </select>
              </label>
              <button
                disabled={busy || !existingId}
                onClick={() => {
                  const room = rooms.find((room) => room.id === existingId && !room.retired);
                  if (room) attach(room);
                }}
              >
                Place selected room
              </button>
            </details>
          )}
        </>
      )}
    </section>
  );
}
