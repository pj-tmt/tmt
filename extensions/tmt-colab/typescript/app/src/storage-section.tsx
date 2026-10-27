import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { relativeTime } from './display-time.js';
import type { StoragePort, StorageRow } from './storage-usage.js';

function size(value: number | null): string {
  if (value === null) return '—';
  if (value < 1000) return `${value} B`;
  if (value < 1000000) return `${(value / 1000).toFixed(1).replace(/\.0$/, '')} KB`;
  if (value < 1000000000) return `${(value / 1000000).toFixed(1).replace(/\.0$/, '')} MB`;
  return `${(value / 1000000000).toFixed(1).replace(/\.0$/, '')} GB`;
}
function CleanupConfirmation({
  title,
  cancel,
  confirm,
}: {
  title: string;
  cancel(): void;
  confirm(): void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const cancelButton = useRef<HTMLButtonElement>(null);
  useLayoutEffect(() => {
    const trigger = document.activeElement;
    const element = dialog.current;
    element?.showModal();
    cancelButton.current?.focus();
    return () => {
      element?.close();
      if (trigger instanceof HTMLElement && trigger.isConnected) trigger.focus();
    };
  }, []);
  return createPortal(
    <dialog
      ref={dialog}
      className="storage-confirm"
      aria-labelledby="storage-confirm-title"
      aria-describedby="storage-confirm-description"
      onCancel={(event) => {
        event.preventDefault();
        event.stopPropagation();
        cancel();
      }}
    >
      <h3 id="storage-confirm-title">Clean up “{title}” now?</h3>
      <p id="storage-confirm-description">
        Deleted text and old edit steps are removed. The page, its comments and attachments stay as
        they are. Other devices catch up from the current version.
      </p>
      <div>
        <button ref={cancelButton} type="button" onClick={cancel}>
          Cancel
        </button>{' '}
        <button type="button" onClick={confirm}>
          Clean up
        </button>
      </div>
    </dialog>,
    document.body,
  );
}

export function StorageSection({
  port,
  pageId,
  title,
}: {
  port: StoragePort;
  pageId: string;
  title: string;
}) {
  const [now] = useState(() => Date.now());
  const [row, setRow] = useState<StorageRow | null>(null);
  const [phase, setPhase] = useState<'loading' | 'ready' | 'confirm' | 'busy'>('loading');
  const [notice, setNotice] = useState('');
  const [error, setError] = useState('');
  const submitting = useRef(false);
  const lifetime = useRef<AbortController | null>(null);
  useEffect(() => {
    const controller = new AbortController();
    lifetime.current = controller;
    port
      .read(pageId, controller.signal)
      .then((next) => {
        if (!controller.signal.aborted) {
          setRow(next);
          setPhase('ready');
        }
      })
      .catch(() => {
        if (!controller.signal.aborted) {
          setError('Storage could not be loaded.');
          setPhase('ready');
        }
      });
    return () => controller.abort();
  }, [port, pageId]);
  async function cleanup() {
    if (submitting.current) return;
    submitting.current = true;
    const signal = lifetime.current?.signal;
    setPhase('busy');
    setError('');
    setNotice('');
    try {
      const result = await port.reclaim(pageId, signal);
      if (signal?.aborted) return;
      if (result.error) {
        const freed = result.freed.history;
        setError(
          freed !== null && freed > 0
            ? `Cleanup failed. Freed ${size(freed)} before the error. The page is unchanged.`
            : 'Cleanup failed. The page is unchanged.',
        );
      } else if (result.skipped) {
        setNotice('Nothing eligible to clean up. The page is unchanged.');
      } else {
        setNotice(`Freed ${size(result.freed.history)}. The page is unchanged.`);
      }
      try {
        const updated = await port.read(pageId, signal);
        if (!signal?.aborted) setRow(updated);
      } catch {
        if (!signal?.aborted) setRow(null);
      }
    } catch {
      if (!signal?.aborted) setError('Cleanup could not be confirmed. The result is unknown.');
    } finally {
      submitting.current = false;
      if (!signal?.aborted) setPhase('ready');
    }
  }
  return (
    <section aria-label="Storage on this Mac">
      <h3>Storage on this Mac</h3>
      {phase === 'loading' && <p role="status">Loading storage…</p>}
      {notice && (
        <p role="status">
          <strong>Done</strong>
          <br />
          <span>{notice}</span>
        </p>
      )}
      {error && <p role="alert">{error}</p>}
      {row && (
        <>
          <dl className="storage-usage">
            <div>
              <dt>Page content</dt>
              <dd>{size(row.contentBytes)}</dd>
            </div>
            <div>
              <dt>Edit history</dt>
              <dd>
                {size(row.historyBytes)}
                {row.freeable.history > 0 && ` (${size(row.freeable.history)} can be freed)`}
              </dd>
            </div>
            <div>
              <dt>Attachments</dt>
              <dd>
                {size(row.attachmentsBytes)}
                {row.unusedAttachmentBytes !== null &&
                  ` (${size(row.unusedAttachmentBytes)} unused)`}
              </dd>
            </div>
            <div>
              <dt>Comments</dt>
              <dd>{size(row.commentsBytes)} (kept)</dd>
            </div>
          </dl>
          <button
            type="button"
            disabled={phase === 'busy' || row.skipped !== null}
            onClick={() => setPhase('confirm')}
          >
            Clean up now
          </button>
          {row.skipped && (
            <p>
              {row.skipped === 'no-tail'
                ? 'There is no new history to clean up.'
                : 'This page is not eligible for cleanup.'}
            </p>
          )}
          <p>Kept: comments, resolved threads, archived state.</p>
          <p>
            Last cleaned automatically{' '}
            {row.lastCleanedAutomaticallyAtMs === null
              ? '—'
              : relativeTime(row.lastCleanedAutomaticallyAtMs, now)}
          </p>
          {phase === 'confirm' && (
            <CleanupConfirmation
              title={title}
              cancel={() => setPhase('ready')}
              confirm={() => void cleanup()}
            />
          )}
          {phase === 'busy' && <p role="status">Cleaning up…</p>}
        </>
      )}
    </section>
  );
}
