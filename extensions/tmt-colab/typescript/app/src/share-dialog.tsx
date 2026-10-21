import { readerUrl } from './reader-link.js';
import { BearerArtifact } from './bearer-artifact.js';
import { DEFAULT_PAGE_EXPIRY_DAYS, localTime } from './expiry.js';
import { text } from './strings.js';
import { RetentionHint } from './retention-hint.js';
import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { useNotificationModalPresence } from './reply-notification-context.js';
import type { payload } from '@tmt/colab-client';
import { PageMenu as BrowserMenu } from './page-menu.js';
import {
  managementFailure,
  newLink,
  type Acknowledgment,
  type ManagementPort,
  type ManagementView,
  type Pending,
  type Selection,
} from './management.js';

interface ManagementOption<Value extends string> {
  value: Value;
  label: string;
  disabled?: boolean;
}

const errors: Record<string, string> = {
  INVALID: 'The request was rejected. Review the selected values.',
  DENIED: 'Management access was denied. Reopen your paired session.',
  EXPIRED: 'This request expired. Refresh and review before signing a new request.',
  STALE_HEAD: 'Space state changed. Refresh and review before signing a new request.',
  CONFLICT:
    'This operation conflicts with stored bytes. Refresh and review; the old request will not be changed.',
  CAPACITY: 'The owner reached a capacity limit. No success has been verified.',
  UNAVAILABLE: 'This operation is unavailable on the owner. No success has been verified.',
  UNKNOWN:
    'Result unknown: the request may have committed. Retry only these exact bytes before expiry.',
};
const audienceOptions: readonly ManagementOption<ManagementView['page']['sharing']>[] = [
  { value: 'private', label: 'Named members only' },
  { value: 'link', label: 'Anyone with the link' },
  { value: 'public', label: 'Anyone on this computer' },
];
const historyOptions: readonly ManagementOption<payload.HistoryMode>[] = [
  { value: 'shared', label: 'Recent history too' },
  { value: 'current', label: 'Current page only' },
];
const roleOptions: readonly ManagementOption<payload.Role>[] = [
  { value: 'viewer', label: 'Read' },
  { value: 'commenter', label: 'Comment' },
  { value: 'editor', label: 'Edit' },
];
export function ShareDialog({
  port,
  pageId,
  title,
  close,
  committed,
}: {
  port: ManagementPort;
  pageId: string;
  title: string;
  close(): void;
  committed(operation: Pending['operation']): void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const heading = useRef<HTMLHeadingElement>(null);
  const submitting = useRef(false);
  const lifetime = useRef<AbortController | null>(null);
  const [forever, setForever] = useState(false);
  const [view, setView] = useState<ManagementView | null>(null);
  const [action, setAction] = useState<{
    selection: Selection;
    label: string;
    detail: string;
    confirmLabel?: string;
  } | null>(null);
  const [pending, setPending] = useState<Pending | null>(null),
    [ack, setAck] = useState<Acknowledgment | null>(null);
  const [phase, setPhase] = useState<
    'loading' | 'ready' | 'confirm' | 'busy' | 'error' | 'verify' | 'done'
  >('loading');
  const [error, setError] = useState(''),
    [unknown, setUnknown] = useState(false);
  const [memberId, setMemberId] = useState(''),
    [signKey, setSignKey] = useState(''),
    [encKey, setEncKey] = useState('');
  const [role, setRole] = useState<payload.Role>('viewer');
  const reportModal = useNotificationModalPresence();
  function showView(next: ManagementView) {
    setView(next);
    setForever(next.page.retentionDays === null);
  }
  useLayoutEffect(() => {
    const trigger = document.activeElement;
    const element = dialog.current;
    element?.showModal();
    reportModal(element?.open ?? false);
    heading.current?.focus({ preventScroll: true });
    return () => {
      element?.close();
      reportModal(false);
      if (trigger instanceof HTMLElement && trigger.isConnected && trigger.getClientRects().length)
        trigger.focus();
      else document.querySelector<HTMLButtonElement>('.page-header-actions button')?.focus();
    };
  }, [reportModal]);
  useEffect(() => {
    let active = true;
    const controller = new AbortController();
    lifetime.current = controller;
    void port
      .read(pageId, controller.signal)
      .then((v) => {
        if (active) {
          showView(v);
          setPhase('ready');
        }
      })
      .catch(() => {
        if (active) {
          setError('Management metadata is unavailable. Refresh to retry.');
          setPhase('error');
        }
      });
    return () => {
      active = false;
      controller.abort();
    };
  }, [port, pageId]);
  async function refresh() {
    setPhase('loading');
    setPending(null);
    setAck(null);
    setAction(null);
    setUnknown(false);
    try {
      const next = await port.read(pageId, lifetime.current?.signal);
      if (lifetime.current?.signal.aborted) return;
      showView(next);
      setError('');
      setPhase('ready');
    } catch {
      if (lifetime.current?.signal.aborted) return;
      setError('Management metadata is unavailable. Refresh to retry.');
      setPhase('error');
    }
  }
  function review(selection: Selection, label: string, detail = '', confirmLabel?: string) {
    setAction({ selection, label, detail, confirmLabel });
    setError('');
    setPhase('confirm');
  }
  async function verify(p: Pending, a: Acknowledgment) {
    setPhase('busy');
    setError('');
    try {
      const next = await port.verify(p, a, lifetime.current?.signal);
      if (lifetime.current?.signal.aborted) return;
      showView(next);
      setPhase('done');
      committed(p.operation);
    } catch {
      if (lifetime.current?.signal.aborted) return;
      setError(
        `${p.operation === 'page.delete' ? 'Deletion' : 'Change'} acknowledged, awaiting verification. Refresh the signed log before claiming completion.`,
      );
      setPhase('verify');
    }
  }
  async function submit() {
    if (!view || !action || submitting.current) return;
    submitting.current = true;
    setPhase('busy');
    setError('');
    setUnknown(false);
    let p = pending;
    try {
      p ??= await port.prepare(view, action.selection);
      if (lifetime.current?.signal.aborted) return;
      setPending(p);
      const a = await port.send(p);
      if (lifetime.current?.signal.aborted) return;
      setAck(a);
      setUnknown(false);
      // An acknowledged management change closes stale preview/writer state.
      committed(p.operation);
      await verify(p, a);
    } catch (e) {
      if (lifetime.current?.signal.aborted) return;
      const failure = managementFailure(e, !!p);
      if (failure.cause instanceof Error) {
        // Only the bounded summary is reported; never the request, stack or original object.
        console.error('Management failed', failure.code, failure.cause.name, failure.cause.message);
      }
      const code = failure.code;
      setError(errors[code] ?? errors.UNKNOWN);
      setUnknown(code === 'UNKNOWN');
      // A lost acknowledgment can still have changed page policy.
      if (p && code === 'UNKNOWN') committed(p.operation);
      setPhase('error');
    } finally {
      submitting.current = false;
    }
  }
  const canClose = phase !== 'busy' && phase !== 'loading';
  return (
    <dialog
      ref={dialog}
      className="management-dialog"
      aria-labelledby="management-title"
      onClose={() => reportModal(false)}
      onClickCapture={(e) => {
        if (!e.isTrusted) e.stopPropagation();
      }}
      onCancel={(e) => {
        e.preventDefault();
        if (canClose && !pending) close();
      }}
    >
      <div className="dialog-bar">
        <h2 ref={heading} id="management-title" tabIndex={-1}>
          {title}
        </h2>
        <button disabled={!canClose} onClick={close}>
          Close
        </button>
      </div>
      <div className="management-dialog-body">
        <p>Share and manage page</p>
        {(phase === 'loading' || phase === 'busy') && (
          <p role="status">
            {phase === 'loading'
              ? 'Loading verified management state…'
              : 'Waiting for the owner and signed log…'}
          </p>
        )}
        {error && <p role="alert">{error}</p>}
        {pending && phase !== 'done' && (
          <p>
            Closing forgets this request and any new sharing secret; the change may already be
            saved. If you lose the secret, use Replace link.
          </p>
        )}
        {phase === 'error' &&
          (unknown && pending ? (
            <button onClick={() => void submit()}>Retry exact request</button>
          ) : (
            <button onClick={() => void refresh()}>Refresh and review</button>
          ))}
        {phase === 'verify' && pending && ack && (
          <button onClick={() => void verify(pending, ack)}>Verify signed log</button>
        )}
        {phase === 'confirm' && action && (
          <section aria-label="Confirm management change">
            <h3>{action.label}</h3>
            <p>{action.detail || 'Apply this change to the selected page?'}</p>
            <p>
              New readers:{' '}
              {view?.page.history === 'shared' ? 'Recent history too' : 'Current page only'}.
              Changes apply at the latest verified revision {view?.revision}.
            </p>
            <button onClick={() => void submit()}>
              {action.confirmLabel ?? `Confirm ${action.label.toLowerCase()}`}
            </button>{' '}
            <button
              onClick={() => {
                setAction(null);
                setPhase('ready');
              }}
            >
              Cancel
            </button>
          </section>
        )}
        {phase === 'done' && (
          <section>
            <p role="status">
              {pending?.operation === 'page.delete' ? 'Deletion' : 'Change'} verified in the signed
              owner log.
              {pending?.operation === 'page.delete' && ack?.storage && (
                <> {text.deletedStorage(ack.storage, ack.storageHint)}</>
              )}
            </p>
            {pending?.artifact && ack && (
              <div className="bearer">
                <p>
                  Copy this link now. It is shown only once and is not saved. Anyone with the link
                  can open this page in a read-only view.
                </p>
                <BearerArtifact
                  key={pending.artifact.linkId}
                  url={readerUrl(location.origin, {
                    space: pending.artifact.space,
                    page: pageId,
                    link: pending.artifact.linkId,
                    revision: ack.membershipHead.revision,
                    statement: ack.membershipHead.statementHash,
                    seed: pending.artifact.seed,
                    root: pending.artifact.root,
                    role: pending.artifact.role,
                  })}
                />
              </div>
            )}
            {view && !view.page.deleted && (
              <button onClick={() => void refresh()}>Manage another change</button>
            )}
          </section>
        )}
        {phase === 'ready' && view && (
          <>
            <p>{view.page.archived ? 'Archived: readable, writes frozen' : 'Active'}</p>
            <details>
              <summary>Details</summary>
              <p className="management-id">Page ID: {pageId}</p>
              <p>Verified revision {view.revision}</p>
              <p>Last edit: {localTime(view.page.lastUpdateAtMs)}</p>
              {view.page.history === 'shared' && (
                <p>Later joins receive the current key and up to 63 earlier keys.</p>
              )}
            </details>
            <section aria-label="Page lifecycle">
              <h3>Retention and lifecycle</h3>
              <RetentionHint page={view.page} />
              <form
                key={view.revision}
                onSubmit={(e) => {
                  e.preventDefault();
                  const days = forever ? null : Number(new FormData(e.currentTarget).get('days'));
                  if (days !== null && (!Number.isSafeInteger(days) || days < 1)) {
                    setError('Retention must be a positive whole number of days or forever.');
                    return;
                  }
                  review(
                    { operation: 'retention.set', value: { pageId, days } },
                    'Set access expiry',
                    days === null
                      ? 'Keep this page on this computer until you delete it.'
                      : `Access expires after ${days} days without a page update. Expiry does not delete your local copy.`,
                  );
                }}
              >
                <label className="checkbox-label">
                  <input
                    type="checkbox"
                    checked={forever}
                    onChange={(e) => setForever(e.target.checked)}
                  />
                  Keep on this computer until I delete it
                </label>
                {!forever && (
                  <label>
                    Days after the last change
                    <input
                      name="days"
                      type="number"
                      min="1"
                      max={Number.MAX_SAFE_INTEGER}
                      step="1"
                      required
                      defaultValue={view.page.retentionDays ?? DEFAULT_PAGE_EXPIRY_DAYS}
                    />
                  </label>
                )}
                <button>Set access expiry</button>
              </form>
              <button
                disabled={view.page.archived}
                onClick={() =>
                  review(
                    { operation: 'page.archive', value: { pageId } },
                    'Archive and stop edits',
                    'Remove this page from the active list and stop new writes. The owner can still read it; there is no restore action here.',
                  )
                }
              >
                Archive and stop edits…
              </button>{' '}
              <button
                onClick={() =>
                  review(
                    { operation: 'page.delete', value: { pageId } },
                    'Delete page',
                    `Stop access and remove stored page data for ${pageId}. Copies already made cannot be taken back, and this page cannot be restored.`,
                  )
                }
              >
                Delete page…
              </button>
            </section>
            <section>
              <h3>Sharing</h3>
              <BrowserMenu
                label="Who can open this page"
                items={audienceOptions.map((option) => ({
                  key: option.value,
                  label: option.label,
                  disabled: option.disabled,
                  checked: option.value === view.page.sharing,
                  role: 'menuitemradio' as const,
                }))}
                valueLabel={
                  audienceOptions.find((option) => option.value === view.page.sharing)?.label ??
                  view.page.sharing
                }
                onSelect={(key) => {
                  const mode = key;
                  if (mode === view.page.sharing) return;
                  const narrowing =
                    (view.page.sharing === 'link' && mode === 'private') ||
                    (view.page.sharing === 'public' && mode !== 'public');
                  review(
                    { operation: 'page.share', value: { pageId, mode } },
                    mode === 'public'
                      ? 'Let anyone on this computer read this page?'
                      : narrowing
                        ? 'Stop access through existing links?'
                        : 'Allow access by link?',
                    mode === 'public'
                      ? `This shares ${view.page.history === 'shared' ? 'recent history too, including deleted text, snapshots, comments and replies' : 'the page as it is now and everything added from now, including comments, replies and attachments'}. Content made public cannot be taken back.`
                      : narrowing
                        ? `Existing links stop working on these pages: ${[...new Set([pageId, ...view.links.flatMap((link) => link.pages)])].sort().join(', ')}; new content is shared only with the remaining audience. Copies already made and content previously made public cannot be taken back; create a new link to share by link again.`
                        : 'Existing links are not reopened. Create a new sharing link for the people you want to invite.',
                    mode === 'public'
                      ? 'Allow anyone on this computer'
                      : narrowing
                        ? 'Stop access through existing links'
                        : 'Allow access by link',
                  );
                }}
              />
              <BrowserMenu
                label="What people who join later see"
                items={historyOptions.map((option) => ({
                  key: option.value,
                  label: option.label,
                  disabled: option.disabled,
                  checked: option.value === view.page.history,
                  role: 'menuitemradio' as const,
                }))}
                valueLabel={
                  historyOptions.find((option) => option.value === view.page.history)?.label ??
                  view.page.history
                }
                onSelect={(mode) => {
                  if (mode === view.page.history) return;
                  review(
                    {
                      operation: 'page.history',
                      value: { pageId, mode },
                    },
                    'Change history',
                    'Applies to people who join later. Access already given stays.',
                  );
                }}
              />
              <p>Applies to people who join later. Access already given stays.</p>
              <p>
                {view.page.history === 'shared'
                  ? 'New readers also get recent history, including deleted text, snapshots and comments. Very old history is not included. Nothing already shared is taken back.'
                  : 'A link reads everything since the last history start, including comments and replies. A new member starts from the page as it is now.'}
              </p>
              <button
                onClick={() =>
                  review(
                    { operation: 'epoch.advance', value: { pageId } },
                    "Start the page's history from now?",
                    'The page keeps its current content. People who join later with "Current page only" see it from here on. Existing links keep working, and anything already shared stays shared.',
                    'Start new history',
                  )
                }
              >
                Start new history…
              </button>
            </section>
            <section>
              <h3>Members and links</h3>
              <p>
                Editors can change what this page’s scripts do for every viewer. A page can navigate
                its own frame; complete exfiltration prevention is not guaranteed.
              </p>
              {view.members.map((m) => (
                <div className="recipient" key={m.id}>
                  <span>
                    {m.id}
                    {m.id === view.ownerMember ? ' (owner management member)' : ''}
                  </span>
                  {m.id !== view.ownerMember && (
                    <>
                      <BrowserMenu
                        label="What this member can do"
                        items={roleOptions.map((option) => ({
                          key: option.value,
                          label: option.label,
                          disabled: option.disabled,
                          checked: option.value === m.role,
                          role: 'menuitemradio' as const,
                        }))}
                        valueLabel={
                          roleOptions.find((option) => option.value === m.role)?.label ?? m.role
                        }
                        onSelect={(role) => {
                          if (role === m.role) return;
                          review(
                            {
                              operation: 'member.role',
                              value: {
                                memberId: m.id,
                                pages: m.pages,
                                role,
                              },
                            },
                            'Change member access',
                            `Give this member ${roleOptions.find((option) => option.value === role)!.label.toLowerCase()} access on ${m.pages.join(', ')}. Content already received cannot be taken back.`,
                          );
                        }}
                      />
                      <button
                        onClick={() =>
                          review(
                            {
                              operation: 'member.remove',
                              value: { memberId: m.id, pages: m.pages },
                            },
                            "Remove this member's access",
                            `Stop this membership on ${m.pages.join(', ')} and keep future content for the remaining people. Earlier copies cannot be taken back, and a separate link they still hold can continue to grant access.`,
                          )
                        }
                      >
                        Remove this member’s access…
                      </button>
                    </>
                  )}
                </div>
              ))}
              <form
                onSubmit={(e) => {
                  e.preventDefault();
                  review(
                    {
                      operation: 'member.add',
                      value: { memberId, role, signKey, encKey, pages: [pageId] },
                    },
                    'Give this member access',
                    `Give ${roleOptions.find((option) => option.value === role)!.label.toLowerCase()} access to this page with ${view.page.history === 'shared' ? 'recent history too' : 'the page as it is now'}. This gives page access, not access to send requests to your agents.`,
                  );
                }}
              >
                {(
                  [
                    ['Member ID', memberId, setMemberId],
                    ['Signing public key', signKey, setSignKey],
                    ['Encryption public key', encKey, setEncKey],
                  ] as const
                ).map(([label, value, change]) => (
                  <label key={label}>
                    {label}
                    <input required value={value} onChange={(e) => change(e.target.value)} />
                  </label>
                ))}
                <BrowserMenu<payload.Role>
                  label="Permission for this new recipient"
                  items={roleOptions.map((option) => ({
                    key: option.value,
                    label: option.label,
                    disabled: option.disabled,
                    checked: option.value === role,
                    role: 'menuitemradio' as const,
                  }))}
                  valueLabel={roleOptions.find((option) => option.value === role)?.label ?? role}
                  onSelect={setRole}
                />
                <button>Give this member access…</button>
              </form>
              <button
                disabled={view.page.sharing === 'private'}
                onClick={() =>
                  review(
                    { operation: 'link.add', value: newLink(role, [pageId]) },
                    'Create a sharing link',
                    `Anyone with this link gets ${roleOptions.find((option) => option.value === role)!.label.toLowerCase()} access to ${view.page.history === 'shared' ? 'the page and recent history' : 'everything since the last history start'}; the browser opens it read-only whatever its permission. You can copy the link once, right after it is created.`,
                  )
                }
              >
                Create a sharing link…
              </button>
              {view.links.map((l) => (
                <div className="recipient" key={l.id}>
                  <span>
                    {l.id} ({roleOptions.find((option) => option.value === l.role)?.label ?? l.role}
                    )
                  </span>
                  <button
                    onClick={() =>
                      review(
                        {
                          operation: 'link.remove',
                          value: {
                            linkId: l.id,
                            pages: l.pages,
                            replacement: newLink(l.role, l.pages),
                          },
                        },
                        'Replace link',
                        `The old link and devices opened from it lose access on ${l.pages.join(', ')}. Share the new link only with intended readers.`,
                      )
                    }
                  >
                    Replace link…
                  </button>
                  <button
                    onClick={() =>
                      review(
                        {
                          operation: 'link.remove',
                          value: { linkId: l.id, pages: l.pages, replacement: null },
                        },
                        'Stop access through this link',
                        `The link and devices opened from it lose access on ${l.pages.join(', ')}; no replacement link is created. Earlier copies cannot be taken back.`,
                      )
                    }
                  >
                    Stop access through this link…
                  </button>
                </div>
              ))}
              <p>
                Removing a device does not stop someone who still has a working sharing link;
                replace that link to exclude its holder. Remote device and agent access are separate
                controls.
              </p>
            </section>
          </>
        )}
      </div>
    </dialog>
  );
}
