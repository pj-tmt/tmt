import type { FurnitureActions } from '../blocks/block-scene.js';
import type { CSSProperties } from 'react';
import type { SelectionAnchor } from '../rendering/selection-anchor.js';
import './furniture-toolbar.css';

const COMMANDS = [
  ['left', '←', 'Move furniture left'],
  ['up', '↑', 'Move furniture up'],
  ['down', '↓', 'Move furniture down'],
  ['right', '→', 'Move furniture right'],
  ['rotate', '↻', 'Rotate selected furniture'],
  ['remove', '×', 'Remove selected furniture'],
] as const;

export function FurnitureToolbar({
  actions,
  anchor,
}: {
  actions: FurnitureActions;
  anchor: SelectionAnchor;
}) {
  return (
    <div
      className="furniture-toolbar"
      role="group"
      aria-label="Selected furniture"
      style={
        { '--selection-x': `${anchor.x}px`, '--selection-y': `${anchor.y}px` } as CSSProperties
      }
    >
      <span className="furniture-toolbar-label" title={actions.label}>
        {actions.label}
      </span>
      <div className="furniture-toolbar-actions">
        {COMMANDS.map(([key, icon, label]) => (
          <button
            key={key}
            aria-label={label}
            title={label}
            disabled={!actions[key]}
            onClick={actions[key]}
          >
            {icon}
          </button>
        ))}
      </div>
    </div>
  );
}
