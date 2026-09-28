import { useEffect, useMemo, useRef } from 'react';
import type { AreaPopulation } from '../local/office-population.js';
import type { WorldDocument } from './world-contract.js';
import { AreaRemovalPreview } from './area-removal-preview.js';
import { previewModuleRemoval } from './module-authoring.js';

/** Native dialog owns focus/escape; history and persistence remain with the editor. */
export function ModuleRemovalDialog({
  world,
  population,
  busy,
  cancel,
  confirm,
}: {
  world: WorldDocument;
  population: AreaPopulation;
  busy: boolean;
  cancel(): void;
  confirm(): boolean;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const cancelButton = useRef<HTMLButtonElement>(null);
  const removal = useMemo(
    () => previewModuleRemoval(world, population.area.id),
    [world, population.area.id]
  );
  useEffect(() => {
    dialog.current?.showModal();
    cancelButton.current?.focus({ preventScroll: true });
  }, []);
  function dismiss() {
    if (busy) return;
    // Close while mounted so the browser restores focus to the opener. Removing
    // an open dialog from the React tree skips that native close lifecycle.
    dialog.current?.close();
    cancel();
  }
  return (
    <dialog
      ref={dialog}
      className="world-removal-dialog"
      aria-label={`Remove ${population.area.name} module`}
      onKeyDown={(event) => {
        if (event.key === 'Escape') event.stopPropagation();
      }}
      onCancel={(event) => {
        event.preventDefault();
        dismiss();
      }}
    >
      <div className="world-removal-details">
        <AreaRemovalPreview world={world} population={population} removal={removal} />
      </div>
      <div className="world-removal-actions">
        <button ref={cancelButton} disabled={busy} onClick={dismiss}>
          Cancel removal
        </button>
        <button
          disabled={busy || Boolean(removal.reason)}
          onClick={() => {
            if (confirm()) dialog.current?.close();
          }}
        >
          Remove module
        </button>
      </div>
    </dialog>
  );
}
