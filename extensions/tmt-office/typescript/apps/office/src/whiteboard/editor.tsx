import { useContext, useEffect, useState, useSyncExternalStore } from 'react';
import { Link, useBlocker } from '@tanstack/react-router';
import { LocalRuntimeContext } from '../local/local-runtime.js';
import { createWhiteboardState } from './editor-state.js';
import type { WhiteboardState, WhiteboardLeaveState } from './editor-state.js';
import { currentScene } from './history.js';
import { WhiteboardCanvas } from './editor-canvas.js';
import type { WhiteboardTool } from './editor-canvas.js';
import type { TextElement } from './scene-contract.js';
import { WhiteboardSnapshotReview } from './snapshot-review.js';
import './whiteboard.css';

const tools: readonly [WhiteboardTool, string][] = [
  ['select', 'Select'],
  ['stroke', 'Pen'],
  ['note', 'Note'],
  ['text', 'Text'],
  ['arrow', 'Arrow'],
  ['rectangle', 'Box'],
  ['ellipse', 'Ellipse'],
  ['erase', 'Erase'],
];

interface EditorProps {
  documentId: string;
  onLeaveStateChange?(state: WhiteboardLeaveState): void;
}

/** The same document editor can be hosted in a spatial panel or a direct local route. */
export function WhiteboardEditor({ documentId, onLeaveStateChange }: EditorProps) {
  const runtime = useContext(LocalRuntimeContext);
  const [state, setState] = useState<WhiteboardState>();
  useEffect(() => {
    if (!runtime) return;
    const owner = createWhiteboardState(runtime.whiteboards, documentId);
    setState(owner);
    void owner.load();
    return () => owner.dispose();
  }, [runtime, documentId]);
  if (!runtime) return <p role="alert">Start a local Office session to open this whiteboard.</p>;
  if (!state) return <p role="status">Opening whiteboard…</p>;
  return <EditorSession key={documentId} state={state} onLeaveStateChange={onLeaveStateChange} />;
}

function EditorSession({
  state,
  onLeaveStateChange,
}: { state: WhiteboardState } & Pick<EditorProps, 'onLeaveStateChange'>) {
  const snapshot = useSyncExternalStore(state.subscribe, state.getSnapshot);
  const [tool, setTool] = useState<WhiteboardTool>('select');
  const [color, setColor] = useState('#285954');
  const [selected, select] = useState<string>();
  const [review, setReview] = useState(false);
  const [textDirty, setTextDirty] = useState(false);
  const [reviewLeaveState, setReviewLeaveState] = useState<WhiteboardLeaveState>('ready');
  useEffect(() => {
    // Read current owner state on every publication rather than relying on a
    // later render to protect in-flight and unconfirmed saves.
    const report = () => {
      const current = state.getSnapshot();
      onLeaveStateChange?.(
        current.busy || current.unconfirmed || reviewLeaveState === 'pending'
          ? 'pending'
          : current.dirty || textDirty || reviewLeaveState === 'draft'
            ? 'draft'
            : 'ready'
      );
    };
    report();
    return state.subscribe(report);
  }, [state, textDirty, reviewLeaveState, onLeaveStateChange]);
  const history = snapshot.history;
  if (!history)
    return (
      <section className="whiteboard-editor">
        <h1>Whiteboard</h1>
        <p role={snapshot.error ? 'alert' : 'status'}>{snapshot.error ?? 'Opening whiteboard…'}</p>
        {!snapshot.busy && <button onClick={() => void state.load()}>Try again</button>}
      </section>
    );
  const scene = currentScene(history);
  const element = scene.elements.find((item) => item.id === selected);
  const disabled = snapshot.busy || snapshot.unconfirmed;
  return (
    <section
      className="whiteboard-editor"
      aria-label="Whiteboard editor"
      onKeyDown={(event) => {
        if (review) return;
        if (event.target instanceof HTMLInputElement || event.target instanceof HTMLTextAreaElement)
          return;
        if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'z') {
          event.preventDefault();
          if (event.shiftKey) state.redo();
          else state.undo();
        }
      }}
    >
      <header className="whiteboard-heading">
        <div>
          <span className="whiteboard-kicker">COMMONS / THINKING SPACE</span>
          <h1>Whiteboard</h1>
        </div>
        <div className="whiteboard-save">
          <button aria-pressed={review} onClick={() => setReview(!review)}>
            {review ? 'Back to drawing' : 'Review snapshot'}
          </button>
          <span role="status">
            {snapshot.busy
              ? 'Working…'
              : snapshot.unconfirmed
                ? 'Draft kept · save unconfirmed'
                : snapshot.dirty
                  ? 'Unsaved changes'
                  : snapshot.document?.revision === 0
                    ? 'New board · not saved'
                    : `Saved · revision ${snapshot.document?.revision}`}
          </span>
          <button
            className="whiteboard-primary"
            disabled={
              snapshot.busy ||
              (!snapshot.dirty && !snapshot.unconfirmed && snapshot.document?.revision !== 0)
            }
            onClick={() => void state.save()}
          >
            {snapshot.unconfirmed ? 'Retry save' : 'Save'}
          </button>
        </div>
      </header>
      {snapshot.error && (
        <div className="whiteboard-error">
          <p role="alert">{snapshot.error}</p>
          <button disabled={snapshot.busy} onClick={() => void state.load()}>
            Discard draft and reload
          </button>
        </div>
      )}
      <div hidden={review}>
        <div className="whiteboard-toolbar" role="toolbar" aria-label="Drawing tools">
          {tools.map(([value, label]) => (
            <button
              key={value}
              disabled={disabled}
              aria-pressed={tool === value}
              onClick={() => setTool(value)}
            >
              {label}
            </button>
          ))}
          <label className="whiteboard-color">
            Ink
            <input
              type="color"
              aria-label="Ink color"
              value={color}
              disabled={disabled}
              onChange={(event) => setColor(event.target.value)}
            />
          </label>
          <button disabled={disabled || history.cursor === 0} onClick={state.undo}>
            Undo
          </button>
          <button
            disabled={disabled || history.cursor === history.entries.length - 1}
            onClick={state.redo}
          >
            Redo
          </button>
        </div>
        <div className="whiteboard-workspace">
          <WhiteboardCanvas
            scene={scene}
            tool={tool}
            color={color}
            disabled={disabled}
            selected={selected}
            select={select}
            commit={state.edit}
          />
          <aside className="whiteboard-inspector" aria-label="Whiteboard elements">
            <h2>{element ? 'Selected element' : 'On this board'}</h2>
            {element && 'text' in element && (
              <TextEditor
                key={`${element.id}:${element.text}`}
                element={element}
                disabled={disabled}
                onDirtyChange={setTextDirty}
                apply={(text) =>
                  state.edit({
                    ...scene,
                    elements: scene.elements.map((item) =>
                      item.id === element.id ? { ...element, text } : item
                    ),
                  })
                }
              />
            )}
            {element && (
              <button
                disabled={disabled}
                onClick={() => {
                  state.edit({
                    ...scene,
                    elements: scene.elements.filter((item) => item.id !== element.id),
                  });
                  select(undefined);
                }}
              >
                Delete selected
              </button>
            )}
            <p>{scene.elements.length} elements · click to select</p>
            <ol>
              {scene.elements.map((item, index) => (
                <li key={item.id}>
                  <button
                    aria-pressed={selected === item.id}
                    onClick={() => {
                      select(item.id);
                      setTool('select');
                    }}
                  >
                    {index + 1}. {item.kind}
                    {'text' in item && item.text ? ` — ${item.text.slice(0, 36)}` : ''}
                  </button>
                </li>
              ))}
            </ol>
            <p className="whiteboard-tip">
              Drag to draw or move. Escape cancels a drag. Notes and text can be edited here.
            </p>
          </aside>
        </div>
      </div>
      <div hidden={!review}>
        <WhiteboardSnapshotReview
          document={snapshot.document!}
          disabled={disabled || snapshot.dirty}
          selected={selected}
          onLeaveStateChange={setReviewLeaveState}
        />
      </div>
    </section>
  );
}

function TextEditor({
  element,
  disabled,
  apply,
  onDirtyChange,
}: {
  element: TextElement;
  disabled: boolean;
  apply(text: string): void;
  onDirtyChange(dirty: boolean): void;
}) {
  const [text, setText] = useState(element.text);
  useEffect(() => {
    onDirtyChange(text !== element.text);
    return () => onDirtyChange(false);
  }, [text, element.text, onDirtyChange]);
  return (
    <form
      onSubmit={(event) => {
        event.preventDefault();
        apply(text);
      }}
    >
      <label>
        Text
        <textarea
          value={text}
          disabled={disabled}
          onChange={(event) => setText(event.target.value)}
        />
      </label>
      <button disabled={disabled || text === element.text}>Apply text</button>
    </form>
  );
}

export function LocalWhiteboardPage({ documentId }: { documentId: string }) {
  const [leaveState, setLeaveState] = useState<WhiteboardLeaveState>('ready');
  const blocker = useBlocker({
    shouldBlockFn: () => leaveState !== 'ready',
    enableBeforeUnload: leaveState !== 'ready',
    withResolver: true,
  });
  return (
    <div className="whiteboard-page">
      <Link to="/local">← Back to office</Link>
      {blocker.status === 'blocked' && (
        <section aria-label="Leave whiteboard confirmation">
          <h2>Leave this whiteboard?</h2>
          <p>
            {leaveState === 'pending'
              ? 'Resolve the current operation before leaving. Submitted work may already have been applied.'
              : 'Leaving discards the unsaved draft. Saved content is not deleted.'}
          </p>
          <button
            disabled={leaveState === 'pending'}
            onClick={() => {
              if (leaveState !== 'pending') blocker.proceed();
            }}
          >
            Discard draft and leave
          </button>
          <button onClick={blocker.reset}>Stay on this whiteboard</button>
        </section>
      )}
      <WhiteboardEditor
        key={documentId}
        documentId={documentId}
        onLeaveStateChange={setLeaveState}
      />
    </div>
  );
}
