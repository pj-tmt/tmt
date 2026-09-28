import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import type { ReactNode, RefObject } from 'react';
import type { CatalogPlacement } from '../rendering/catalog-placement.js';
import { FurnitureToolbar } from './furniture-toolbar.js';
import type { FurnitureActions } from '../blocks/block-scene.js';
import { sameSelectionTarget } from '../rendering/selection-anchor.js';
import type { SelectionAnchor, SelectionTarget } from '../rendering/selection-anchor.js';
import type { OfficeSelection } from '../rendering/office-selection.js';
import type { MeetingSlot } from '../world-map/module-contract.js';
import type {
  OfficeSceneModel,
  OfficeSceneEditor,
  createOfficeScene,
} from '../rendering/office-scene.js';

type Scene = NonNullable<Awaited<ReturnType<typeof createOfficeScene>>>;

export function OfficeCanvas({
  model,
  selection,
  select,
  editor,
  activate,
  focusedComponentId,
  furnitureActions,
  officeOverlay,
  meetingOverlay,
  createMeeting,
  cameraHost,
  placementRef,
}: {
  model: OfficeSceneModel;
  selection?: OfficeSelection;
  select: (selection: OfficeSelection) => void;
  editor?: OfficeSceneEditor;
  activate?: (id: string) => void;
  focusedComponentId?: string;
  furnitureActions?: FurnitureActions;
  officeOverlay?(anchor?: SelectionTarget): ReactNode;
  meetingOverlay?(anchor?: SelectionTarget): ReactNode;
  createMeeting?(slot: MeetingSlot): void;
  cameraHost?: HTMLElement | null;
  placementRef?: RefObject<CatalogPlacement | undefined>;
}) {
  const host = useRef<HTMLDivElement>(null);
  const scene = useRef<Scene | undefined>(undefined);
  const latest = useRef({
    model,
    selection,
    select,
    editor,
    activate,
    focusedComponentId,
    createMeeting,
  });
  const synchronize = useRef(() => {});
  const [ready, setReady] = useState(false);
  const [error, setError] = useState(false);
  const [anchor, setAnchor] = useState<SelectionAnchor>();
  const [officeAnchor, setOfficeAnchor] = useState<SelectionTarget>();
  const [meetingAnchor, setMeetingAnchor] = useState<SelectionTarget>();
  useEffect(() => {
    latest.current = {
      model,
      selection,
      select,
      editor,
      activate,
      focusedComponentId,
      createMeeting,
    };
    synchronize.current();
  }, [model, selection, select, editor, activate, focusedComponentId, createMeeting]);
  useEffect(() => {
    const element = host.current;
    if (!element) return;
    const controller = new AbortController();
    let applied: typeof latest.current | undefined;
    function fail() {
      if (controller.signal.aborted) return;
      scene.current = undefined;
      if (placementRef) placementRef.current = undefined;
      controller.abort();
      setReady(false);
      setError(true);
      setAnchor(undefined);
    }
    function flush() {
      const current = scene.current;
      if (!current || controller.signal.aborted || document.hidden) return;
      const next = latest.current;
      try {
        if (applied?.model !== next.model) current.update(next.model);
        if (!applied || applied.selection !== next.selection) current.selection(next.selection);
        if (!applied || applied.editor !== next.editor) current.editing(next.editor);
        if (!applied || applied.focusedComponentId !== next.focusedComponentId)
          current.interaction(next.focusedComponentId);
        applied = next;
        setReady(true);
      } catch {
        fail();
      }
    }
    synchronize.current = flush;
    document.addEventListener('visibilitychange', flush);
    void import('../rendering/office-scene.js')
      .then(({ createOfficeScene }) =>
        createOfficeScene(element, controller.signal, {
          select: (id) => {
            if (!controller.signal.aborted) latest.current.select(id);
          },
          activate: (id) => {
            if (!controller.signal.aborted) latest.current.activate?.(id);
          },
          createMeeting: (slot) => {
            if (!controller.signal.aborted) latest.current.createMeeting?.(slot);
          },
          selectedAnchor: (next) => {
            if (!controller.signal.aborted)
              setAnchor((previous) =>
                previous?.x === next?.x && previous?.y === next?.y ? previous : next
              );
          },
          officeAnchor: (next) => {
            if (!controller.signal.aborted)
              setOfficeAnchor((previous) =>
                sameSelectionTarget(previous, next) ? previous : next
              );
          },
          meetingAnchor: (next) => {
            if (!controller.signal.aborted)
              setMeetingAnchor((previous) =>
                sameSelectionTarget(previous, next) ? previous : next
              );
          },
        })
      )
      .then((created) => {
        if (!created) return;
        if (controller.signal.aborted) {
          created.dispose();
          return;
        }
        scene.current = created;
        if (placementRef) placementRef.current = created.catalogPlacement;
        flush();
      })
      .catch(fail);
    return () => {
      synchronize.current = () => {};
      document.removeEventListener('visibilitychange', flush);
      controller.abort();
      if (placementRef) placementRef.current = undefined;
      scene.current = undefined;
    };
  }, [placementRef]);
  const cameraControls = ready && (
    <div className="scene-camera" role="group" aria-label="Map view">
      <button aria-label="Zoom out" title="Zoom out" onClick={() => scene.current?.zoom(1 / 1.2)}>
        −
      </button>
      <button
        className="scene-fit"
        aria-label="Fit office"
        title="Show the whole office"
        onClick={() => scene.current?.fit()}
      >
        Fit
      </button>
      <button aria-label="Zoom in" title="Zoom in" onClick={() => scene.current?.zoom(1.2)}>
        +
      </button>
    </div>
  );
  return (
    <div className="office-map" data-scene-ready={ready}>
      <div
        ref={host}
        className="office-canvas"
        role="img"
        aria-label="Office map. Drag or scroll to pan; pinch to zoom. Use the space directory and object controls for keyboard access."
      />
      {!ready && (
        <p className="scene-status" role={error ? 'alert' : 'status'}>
          {error
            ? 'The map could not initialize. Your rooms are still accessible from the agent directory.'
            : 'Opening the workshop…'}
        </p>
      )}
      {cameraHost ? createPortal(cameraControls, cameraHost) : cameraControls}
      {ready && editor && anchor && furnitureActions && (
        <FurnitureToolbar actions={furnitureActions} anchor={anchor} />
      )}
      {ready && officeOverlay?.(officeAnchor)}
      {ready && meetingOverlay?.(meetingAnchor)}
    </div>
  );
}
