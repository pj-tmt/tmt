import { useEffect, useRef, useState } from 'react';
import type { OfficeSlot } from './module-contract.js';
import { useAnchoredPanel } from './use-anchored-panel.js';
import type { PanelObstacles } from './use-anchored-panel.js';
import type { SelectionTarget } from '../rendering/selection-anchor.js';

export function OfficeExpansionForm({
  selected,
  anchor,
  obstacles,
  busy,
  create,
  cancel,
}: {
  selected: OfficeSlot;
  anchor?: SelectionTarget;
  obstacles?: PanelObstacles;
  busy: boolean;
  create(name: string): void;
  cancel(): void;
}) {
  const [name, setName] = useState('');
  const input = useRef<HTMLInputElement>(null);
  const panel = useAnchoredPanel<HTMLFormElement>(anchor, obstacles);
  useEffect(() => {
    if (selected) input.current?.focus({ preventScroll: true });
  }, [selected]);
  return (
    <form
      ref={panel.ref}
      className="office-expansion-card"
      aria-label="New office"
      style={panel.style}
      onSubmit={(event) => {
        event.preventDefault();
        if (selected && !busy) create(name);
      }}
      onKeyDown={(event) => {
        if (event.key === 'Escape' && !busy) {
          event.stopPropagation();
          cancel();
        }
      }}
    >
      <fieldset disabled={busy}>
        <h2>New office</h2>
        <p>
          Column {selected.column}, row {selected.row}
        </p>
        <label>
          Name
          <input
            ref={input}
            required
            maxLength={80}
            value={name}
            onChange={(event) => setName(event.target.value)}
          />
        </label>
        <div className="office-expansion-actions">
          <button type="button" aria-label="Cancel placement" onClick={cancel}>
            Cancel
          </button>
          <button type="submit" aria-label="Add office" disabled={!selected || !name.trim()}>
            Add
          </button>
        </div>
      </fieldset>
    </form>
  );
}
