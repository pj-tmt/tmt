import { useContext, useEffect, useState, useSyncExternalStore } from 'react';
import { LocalRuntimeContext } from '../local/local-runtime.js';
import type { WhiteboardDocument } from './document-contract.js';
import { createSnapshotState } from './snapshot-state.js';
import type { SnapshotState } from './snapshot-state.js';
import type { WhiteboardLeaveState } from './editor-state.js';
import { renderSnapshotImage } from './snapshot-image.js';
import { SnapshotReferenceView } from './snapshot-reference-view.js';
import { WhiteboardSnapshotSend } from './snapshot-send.js';
import './snapshot-review.css';

interface Props {
  document: WhiteboardDocument;
  disabled: boolean;
  selected?: string;
  onLeaveStateChange?(state: WhiteboardLeaveState): void;
}

export function WhiteboardSnapshotReview(props: Props) {
  const runtime = useContext(LocalRuntimeContext);
  const [state, setState] = useState<SnapshotState>();
  useEffect(() => {
    if (!runtime) return;
    const owner = createSnapshotState(runtime.whiteboardSnapshots, renderSnapshotImage);
    setState(owner);
    return () => owner.dispose();
  }, [runtime]);
  return state ? (
    <SnapshotReviewSession {...props} state={state} />
  ) : (
    <p role="status">Opening snapshot tools…</p>
  );
}

function StoredImage({ image }: { image: Blob }) {
  const [url, setUrl] = useState<string>();
  const [failed, setFailed] = useState(false);
  const [actualSize, setActualSize] = useState(false);
  useEffect(() => {
    const next = URL.createObjectURL(image);
    setUrl(next);
    setFailed(false);
    setActualSize(false);
    return () => URL.revokeObjectURL(next);
  }, [image]);
  return (
    <>
      {url && (
        <>
          <div className="whiteboard-review-image-tools" role="group" aria-label="Snapshot zoom">
            <button aria-pressed={!actualSize} onClick={() => setActualSize(false)}>
              Fit board
            </button>
            <button aria-pressed={actualSize} onClick={() => setActualSize(true)}>
              Actual size
            </button>
          </div>
          <div
            className="whiteboard-review-image-viewport"
            data-actual-size={actualSize}
            role="region"
            aria-label="Snapshot image viewport"
            tabIndex={0}
          >
            <img src={url} alt="Saved whiteboard snapshot" onError={() => setFailed(true)} />
          </div>
        </>
      )}
      {failed && <p role="alert">The stored snapshot image could not be displayed.</p>}
    </>
  );
}

function SnapshotReviewSession({
  document,
  disabled,
  selected,
  state,
  onLeaveStateChange,
}: Props & { state: SnapshotState }) {
  const runtime = useContext(LocalRuntimeContext);
  const snapshot = useSyncExternalStore(state.subscribe, state.getSnapshot);
  const [sendDraft, setSendDraft] = useState(false);
  const [sendPending, setSendPending] = useState(false);
  const [confirmReset, setConfirmReset] = useState(false);
  const [annotation, setAnnotation] = useState('');
  const [selection, setSelection] = useState<string[]>([]);
  const frozen = snapshot.pending || Boolean(snapshot.captured);
  const present = new Set(document.scene.elements.map((element) => element.id));
  const selectedIds = selection.filter((id) => present.has(id));
  const selectedSet = new Set(selectedIds);
  const canCapture = !disabled && document.revision > 0 && !frozen && !snapshot.busy;
  useEffect(() => {
    onLeaveStateChange?.(
      snapshot.busy || snapshot.pending || sendPending
        ? 'pending'
        : sendDraft || (!snapshot.captured && Boolean(annotation || selection.length))
          ? 'draft'
          : 'ready'
    );
  }, [
    snapshot.busy,
    snapshot.pending,
    snapshot.captured,
    sendPending,
    sendDraft,
    annotation,
    selection,
    onLeaveStateChange,
  ]);
  return (
    <section className="whiteboard-review" aria-label="Whiteboard snapshot">
      <div className="whiteboard-review-preview">
        {snapshot.image ? (
          <StoredImage image={snapshot.image} />
        ) : (
          <div className="whiteboard-review-placeholder">
            <span aria-hidden="true">▣</span>
            <h2>{snapshot.busy ? 'Preparing your snapshot…' : 'A fixed view of your ideas'}</h2>
            <p>Keep the board, highlighted elements and your annotation together.</p>
          </div>
        )}
        {snapshot.captured && (
          <p className="whiteboard-review-caption">
            Revision {snapshot.captured.documentRevision} ·{' '}
            {snapshot.captured.selectedElementIds.length
              ? `${snapshot.captured.selectedElementIds.length} highlighted`
              : 'Entire board'}
            {snapshot.image ? ' · Image saved' : ' · Awaiting image'}
          </p>
        )}
      </div>
      <aside className="whiteboard-inspector whiteboard-review-inspector">
        <span className="whiteboard-kicker">CAPTURE / REVIEW</span>
        <h2>{snapshot.image ? 'Snapshot ready' : 'Add context'}</h2>
        {snapshot.captured ? (
          <>
            <p className="whiteboard-review-annotation">
              {snapshot.captured.annotation || 'No annotation.'}
            </p>
            <p>This preview is fixed. Further drawing does not change it.</p>
          </>
        ) : snapshot.intent ? (
          <>
            <p className="whiteboard-review-annotation">
              {snapshot.intent.annotation || 'No annotation.'}
            </p>
            <p>
              Capturing revision {snapshot.intent.expectedRevision} ·{' '}
              {snapshot.intent.selectedElementIds.length
                ? `${snapshot.intent.selectedElementIds.length} highlighted`
                : 'Entire board'}
            </p>
          </>
        ) : (
          <>
            <label>
              Annotation
              <textarea
                value={annotation}
                disabled={frozen}
                onChange={(event) => setAnnotation(event.target.value)}
                placeholder="What should someone look at?"
              />
            </label>
            <fieldset disabled={frozen} className="whiteboard-review-selection">
              <legend>Highlight elements</legend>
              <p>
                None selected includes the entire board. Highlights keep the surrounding drawing
                visible.
              </p>
              {selected && present.has(selected) && (
                <button type="button" onClick={() => setSelection([selected])}>
                  Use selected element
                </button>
              )}
              <ul>
                {document.scene.elements.map((item, index) => (
                  <li key={item.id}>
                    <label>
                      <input
                        type="checkbox"
                        checked={selectedSet.has(item.id)}
                        onChange={(event) =>
                          setSelection(
                            event.target.checked
                              ? [...selectedIds, item.id]
                              : selectedIds.filter((id) => id !== item.id)
                          )
                        }
                      />
                      {index + 1}. {item.kind}
                      {'text' in item && item.text ? ` — ${item.text.slice(0, 36)}` : ''}
                    </label>
                  </li>
                ))}
              </ul>
            </fieldset>
          </>
        )}
        {snapshot.error && <p role="alert">{snapshot.error}</p>}
        {snapshot.image && snapshot.captured && (
          <SnapshotReferenceView
            key={`reference:${snapshot.captured.id}`}
            id={snapshot.captured.id}
          />
        )}
        <div className="whiteboard-review-actions">
          {!frozen && (
            <button
              className="whiteboard-primary"
              disabled={!canCapture}
              onClick={() => void state.capture(document, selectedIds, annotation)}
            >
              Create snapshot
            </button>
          )}
          {snapshot.pending && (
            <button
              className="whiteboard-primary"
              disabled={snapshot.busy}
              onClick={() => void state.retry()}
            >
              Retry snapshot
            </button>
          )}
          {frozen && (
            <button
              disabled={snapshot.busy || sendDraft || sendPending}
              onClick={() => {
                if (snapshot.pending) setConfirmReset(true);
                else state.reset();
              }}
            >
              Start new preview
            </button>
          )}
        </div>
        {confirmReset && (
          <section aria-label="Discard unconfirmed snapshot confirmation">
            <p>
              The snapshot may already have been saved. Starting over discards its retry intent; it
              does not delete saved snapshots or cancel submitted work.
            </p>
            <button
              disabled={snapshot.busy || sendDraft || sendPending}
              onClick={() => {
                state.reset();
                setConfirmReset(false);
              }}
            >
              Discard retry and start over
            </button>
            <button onClick={() => setConfirmReset(false)}>Keep snapshot retry</button>
          </section>
        )}
        {!frozen && (disabled || document.revision === 0) && (
          <p role="status">Save your changes before creating a snapshot.</p>
        )}
        <p className="whiteboard-tip">Creating a snapshot does not send a request.</p>
        {snapshot.image && snapshot.captured && runtime && (
          <WhiteboardSnapshotSend
            key={`send:${snapshot.captured.id}`}
            snapshotId={snapshot.captured.id}
            dispatch={runtime.dispatch}
            profiles={runtime.profiles}
            rooms={runtime.rooms}
            onDraftChange={setSendDraft}
            onPendingChange={setSendPending}
          />
        )}
      </aside>
    </section>
  );
}
