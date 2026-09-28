import { useState } from 'react';
import type { OfficeSlot } from './module-contract.js';
import type { MeetingRoom, RoomPort } from '../local/room-contract.js';
import type { PanelObstacles } from './use-anchored-panel.js';
import { useAnchoredPanel } from './use-anchored-panel.js';
import type { SelectionTarget } from '../rendering/selection-anchor.js';
import { MeetingCreationForm } from './meeting-creation-form.js';

/** One prospective platform, independent of its intended use. */
export function AreaCreationForm({
  selected,
  anchor,
  obstacles,
  busy,
  port,
  rooms,
  onBusyChange,
  roomSaved,
  createOffice,
  createMeeting,
  close,
}: {
  selected: OfficeSlot;
  anchor?: SelectionTarget;
  obstacles?: PanelObstacles;
  busy: boolean;
  port: RoomPort;
  rooms: readonly MeetingRoom[];
  onBusyChange(busy: boolean): void;
  roomSaved(room: MeetingRoom): void;
  createOffice(name: string): void;
  createMeeting(room: MeetingRoom, id: string): boolean;
  close(): void;
}) {
  const [use, setUse] = useState<'office' | 'meeting'>('office');
  const [name, setName] = useState('');
  const panel = useAnchoredPanel<HTMLElement>(anchor, obstacles);
  return (
    <section
      ref={panel.ref}
      style={panel.style}
      className="office-expansion-card area-creation-card"
      aria-label="New area"
      onKeyDown={(event) => {
        if (event.key === 'Escape' && !busy) {
          event.stopPropagation();
          close();
        }
      }}
    >
      <h2>New area</h2>
      <p>
        Column {selected.column}, row {selected.row}
      </p>
      <fieldset disabled={busy} className="area-use-choices">
        <legend>Use</legend>
        <button type="button" aria-pressed={use === 'office'} onClick={() => setUse('office')}>
          Office
        </button>
        <button type="button" aria-pressed={use === 'meeting'} onClick={() => setUse('meeting')}>
          Meeting room
        </button>
      </fieldset>
      {use === 'meeting' ? (
        <MeetingCreationForm
          embedded
          port={port}
          rooms={rooms}
          busy={busy}
          onBusyChange={onBusyChange}
          roomSaved={roomSaved}
          place={createMeeting}
          close={close}
        />
      ) : (
        <form
          onSubmit={(event) => {
            event.preventDefault();
            if (!busy && name.trim()) createOffice(name);
          }}
        >
          <fieldset disabled={busy}>
            <label>
              Area name
              <input
                autoFocus
                required
                maxLength={80}
                value={name}
                onChange={(event) => setName(event.target.value)}
              />
            </label>
            <div className="office-expansion-actions">
              <button type="button" onClick={close}>
                Cancel
              </button>
              <button type="submit" disabled={!name.trim()}>
                Add area
              </button>
            </div>
          </fieldset>
        </form>
      )}
    </section>
  );
}
