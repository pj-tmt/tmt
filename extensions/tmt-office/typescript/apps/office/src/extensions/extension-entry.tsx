import { useId } from 'react';
import type { BoundExtension } from './extension-binding.js';
import './extension-entry.css';

/** Accessible counterpart of an object's spatial action, not another handler. */
export function ExtensionEntry({
  entry,
  activate,
  label,
  focus,
  description,
}: {
  entry: BoundExtension;
  activate: (id: string) => void;
  label?: string;
  focus?: (id?: string) => void;
  description?: string;
}) {
  const descriptionId = useId();
  return (
    <button
      type="button"
      className="world-extension-entry"
      aria-haspopup="dialog"
      aria-label={entry.unavailable ? undefined : (label ?? entry.actionLabel)}
      aria-describedby={description ? descriptionId : undefined}
      disabled={!entry.invoke || Boolean(entry.unavailable)}
      title={entry.unavailable}
      onFocus={() => focus?.(entry.instance.id)}
      onBlur={() => focus?.()}
      onClick={() => activate(entry.instance.id)}
    >
      <span className="world-extension-action-mark" aria-hidden="true">
        {entry.unavailable ? '×' : '↗'}
      </span>
      <span className="world-extension-copy">
        <span>
          {entry.unavailable ? `${entry.label} — ${entry.unavailable}` : (label ?? entry.label)}
        </span>
        {description && <small id={descriptionId}>{description}</small>}
      </span>
    </button>
  );
}
