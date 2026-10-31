import type { IssuedInvitation } from './invitations.js';
import type { IssuedInvitationsPage } from './issued-invitations-page.js';
import { createListTable } from '@tmt/browser-ui/list-table';
import { createIcon } from '@tmt/browser-ui/glyph';
import { mountBrowserMenu, type BrowserMenuHost } from '@tmt/browser-ui/menu-host';
import { browserUiClasses } from '@tmt/browser-ui/static';

type InvitationState = IssuedInvitation['state'];
const STATES: Record<InvitationState, { label: string; tone?: 'working' | 'waiting' | 'blocked' }> =
  {
    open: { label: 'Open', tone: 'working' },
    'awaiting-owner': { label: 'Awaiting you', tone: 'waiting' },
    confirmed: { label: 'Confirmed' },
    refused: { label: 'Refused' },
    cancelled: { label: 'Cancelled' },
    expired: { label: 'Expired', tone: 'blocked' },
    // Amber and labelled: these outcomes need the owner to check what happened.
    interrupted: { label: 'Interrupted · needs action', tone: 'waiting' },
    unconfirmed: { label: 'Unconfirmed · needs action', tone: 'waiting' },
  };
const ACTIVE: readonly InvitationState[] = ['open', 'awaiting-owner'];
/** Bounded copy for the contract's create and cancel refusal reasons. */
const REFUSALS: Record<string, string> = {
  REMOTE_INVITATION_BUSY:
    'Another invitation request is in progress on this machine. Try again when it finishes.',
  REMOTE_INVITATION_CAPACITY: 'This Remote has too many open invitations. Cancel one first.',
  REMOTE_INVITATION_STOPPED:
    'Invitations are stopped on this machine. Start them with tmt remote invite.',
  REMOTE_INVITATION_UNAVAILABLE:
    'Invitations are unavailable right now. Check this Remote on this machine.',
  REMOTE_MANAGEMENT_READ_ONLY: 'This browser is read-only for this Remote.',
  REMOTE_INTENT_CONFLICT: 'This request conflicts with an earlier one. Refresh, then try again.',
};

/** The short label names an invitation; the full offer ID stays in its title and copy action. */
export function invitationLabel(offerId: string): string {
  return `inv-${offerId.slice(0, 4)}`;
}
function ago(at: number, now: number): string {
  const minutes = Math.max(0, Math.floor((now - at) / 60_000));
  if (minutes < 1) return 'Just now';
  if (minutes < 60) return `${minutes} min ago`;
  if (minutes < 1440) return `${Math.floor(minutes / 60)} h ago`;
  return minutes < 2880 ? 'Yesterday' : `${Math.floor(minutes / 1440)} days ago`;
}

/** Mount Invites around the retained issued-invitations model. The created code lives only in
 * the open dialog and the model's memory; closing the dialog clears both. */
export function mountInvitesPage(
  root: HTMLElement,
  page: IssuedInvitationsPage,
  signal: AbortSignal,
  options: {
    otherBusy: () => boolean;
    changed: () => void;
    /** Reveal the existing four-word confirmation for an invitation awaiting the owner. */
    confirm: () => void;
    now?: () => number;
  },
): {
  render: () => void;
  refresh: () => Promise<void>;
  invite: () => Promise<void>;
  heldBy: () => string;
} {
  const now = options.now ?? Date.now;
  const node = <K extends keyof HTMLElementTagNameMap>(tag: K, text = '') => {
    const value = document.createElement(tag);
    value.textContent = text;
    return value;
  };
  const button = (text: string) => {
    const value = node('button', text);
    value.type = 'button';
    value.className = browserUiClasses.action;
    return value;
  };
  const status = node('p', 'Checking invitations…');
  status.setAttribute('role', 'status');
  const facts = node('p');
  const refreshButton = button('Refresh invitations');
  const table = createListTable(
    [
      { id: 'icon', label: '', hideBelow: 'phone' },
      { id: 'invitation', label: 'Invitation' },
      { id: 'status', label: 'Status' },
      { id: 'issued', label: 'Issued', hideBelow: 'phone' },
      { id: 'expires', label: 'Expires' },
      { id: 'actions', label: '' },
    ],
    'Invitations',
    'remote-invites-table remote-phone-rows',
  );
  const footer = node('p');
  footer.className = 'invites-list-footer';
  const first = button('First invitations');
  const more = button('Next invitations');
  const original = node('p');
  const recover = button('Check original outcome');
  root.replaceChildren(
    status,
    facts,
    refreshButton,
    table.root,
    footer,
    first,
    more,
    original,
    recover,
  );
  let menus: BrowserMenuHost<string>[] = [];
  let localNotice = '';
  let dialog: HTMLDialogElement | undefined;

  function heldBy(): string {
    if (page.busy) return 'An invitation request is in progress. Wait for its outcome.';
    if (page.outcome?.state === 'unknown')
      return 'An invitation change is unconfirmed. Check its original outcome first.';
    return '';
  }
  function available(): boolean {
    return (
      page.access === 'live' &&
      !page.busy &&
      !options.otherBusy() &&
      page.outcome?.state !== 'unknown'
    );
  }
  async function run(action: () => Promise<void>): Promise<void> {
    const pending = action();
    render();
    options.changed();
    try {
      await pending;
    } catch {
      localNotice = 'This action could not start. Review current access and try again.';
    }
    if (!signal.aborted) {
      render();
      options.changed();
    }
  }
  async function refresh(cursor?: string | null): Promise<void> {
    if (page.busy || options.otherBusy()) return;
    localNotice = '';
    await run(() => page.refresh(cursor));
  }
  async function invite(): Promise<void> {
    if (!available()) return;
    localNotice = '';
    await run(() => page.create(crypto.randomUUID()));
    if (page.secret) showSecret(page.secret);
    if (page.outcome?.state === 'committed') await refresh(null);
  }
  async function cancel(offerId: string): Promise<void> {
    if (!available()) return;
    localNotice = '';
    await run(() => page.cancel(crypto.randomUUID(), offerId));
    if (page.outcome?.state === 'committed') await refresh(null);
  }
  function copyRow(text: string, label: string): HTMLElement {
    const row = node('div');
    row.className = 'invite-secret-row';
    const value = node('code', text);
    value.className = 'invite-secret';
    const copy = button(label);
    const feedback = node('span');
    feedback.setAttribute('role', 'status');
    copy.addEventListener('click', async () => {
      try {
        await navigator.clipboard.writeText(text);
        feedback.textContent = 'Copied.';
      } catch {
        feedback.textContent = 'Copy failed. Select the text and copy it manually.';
      }
    });
    row.append(value, copy, feedback);
    return row;
  }
  function showSecret(secret: NonNullable<IssuedInvitationsPage['secret']>): void {
    dialog?.close();
    const box = node('dialog');
    box.className = `${browserUiClasses.dialog} invite-dialog`;
    const header = node('header');
    header.className = browserUiClasses.dialogHeader;
    const heading = node('h2', 'Invitation created');
    heading.id = 'invite-dialog-heading';
    box.setAttribute('aria-labelledby', heading.id);
    const close = node('button');
    close.type = 'button';
    close.className = 'tmt-ui-icon-action';
    close.setAttribute('aria-label', 'Close');
    close.append(createIcon('x'));
    header.append(heading, close);
    const body = node('div');
    body.className = browserUiClasses.dialogBody;
    const expiry =
      page.outcome?.state === 'committed' && 'expiresAtMs' in page.outcome.result.invitation
        ? Math.max(1, Math.round((page.outcome.result.invitation.expiresAtMs - now()) / 60_000))
        : undefined;
    body.append(
      node(
        'p',
        `Give this code or link to one person. It works once${expiry ? ` and expires in ${expiry} min` : ''}.`,
      ),
      copyRow(secret.code, 'Copy code'),
      ...(secret.link ? [copyRow(secret.link, 'Copy link')] : []),
      node('p', "This code is shown only now. After you close this, it can't be shown again."),
    );
    const actions = node('footer');
    actions.className = browserUiClasses.dialogActions;
    const done = button('Done');
    done.dataset.variant = 'primary';
    actions.append(done);
    box.append(header, body, actions);
    for (const end of [close, done]) end.addEventListener('click', () => box.close());
    box.addEventListener(
      'close',
      () => {
        // The code never outlives this dialog: no DOM node, model field or storage keeps it.
        box.remove();
        page.clearSecret();
        if (dialog === box) dialog = undefined;
        render();
      },
      { once: true },
    );
    dialog = box;
    document.body.append(box);
    box.showModal();
  }
  signal.addEventListener('abort', () => dialog?.close(), { once: true });
  recover.addEventListener('click', () => void run(() => page.recover()), { signal });
  refreshButton.addEventListener('click', () => void refresh(null), { signal });
  first.addEventListener('click', () => void refresh(null), { signal });
  more.addEventListener('click', () => void refresh(page.history?.nextCursor ?? null), { signal });

  function outcomeText(): string {
    const outcome = page.outcome;
    if (!outcome || !page.intent) return '';
    if (outcome.state === 'unknown')
      return page.intent.kind === 'create'
        ? 'Creating this invitation is unconfirmed. Check its original outcome before inviting again.'
        : 'Cancelling this invitation is unconfirmed. Check its original outcome first.';
    if (outcome.state === 'refused')
      return (
        REFUSALS[outcome.reason] ??
        'The request was refused. Review current access or use tmt remote invite on this machine.'
      );
    const invitation = outcome.result.invitation;
    if (invitation.state === 'created')
      return page.secret
        ? 'Invitation created.'
        : 'Invitation created. Its code is no longer shown; invite again for a new one.';
    if (invitation.state === 'not-active')
      return 'That invitation was no longer open. Nothing changed.';
    if (invitation.state === 'cancelled')
      return 'Invitation cancelled. Anyone who already joined keeps access.';
    return '';
  }
  function render(): void {
    if (signal.aborted) return;
    const view = page.history;
    status.textContent =
      localNotice ||
      outcomeText() ||
      (page.access === 'live'
        ? 'Invitations from the last 7 days. A status describes the invitation, not whether the person is online.'
        : page.access === 'checking'
          ? 'Checking invitations…'
          : 'Invitations could not be confirmed. Use tmt remote invite on this machine.');
    facts.textContent =
      view && !view.invitations.length ? 'No invitations in the last 7 days.' : '';
    facts.hidden = !facts.textContent;
    original.textContent =
      page.outcome?.state === 'unknown' && page.intent
        ? `Original change ${page.intent.operationId}`
        : '';
    original.hidden = !original.textContent;
    recover.hidden = page.outcome?.state !== 'unknown';
    recover.disabled = !page.canRecover || options.otherBusy();
    refreshButton.disabled = page.busy || options.otherBusy();
    first.hidden = page.onFirstPage;
    more.hidden = !view?.nextCursor;
    first.disabled = more.disabled = !available();
    footer.replaceChildren(
      node(
        'span',
        view
          ? `${view.invitations.length} ${view.invitations.length === 1 ? 'invitation' : 'invitations'} · newest first`
          : '',
      ),
      node('span', 'Cancel an open invitation in the row menu'),
    );
    footer.hidden = !view;
    menus.forEach((menu) => menu.dispose());
    menus = [];
    table.body.replaceChildren(
      ...(view?.invitations ?? []).map((invitation) => {
        const leading = node('span');
        leading.className = browserUiClasses.listLeading;
        leading.append(createIcon('link'));
        const title = node('div');
        title.className = browserUiClasses.listTitle;
        title.dataset.phone = 'name';
        const label = node('span', invitationLabel(invitation.offerId));
        label.className = `${browserUiClasses.listLabel} invite-id`;
        label.title = invitation.offerId;
        title.append(label);
        if (invitation.state === 'awaiting-owner') {
          const hint = node('span', 'Someone used this code · confirm or refuse in the row menu');
          hint.className = browserUiClasses.listDescription;
          title.append(hint);
        }
        const state = STATES[invitation.state];
        const chip = node('span');
        chip.className = `${browserUiClasses.chip} remote-stage`;
        if (state.tone) chip.dataset.tone = state.tone;
        const mark = node('span');
        mark.className = browserUiClasses.stateMark;
        mark.dataset.shape = 'dot';
        mark.setAttribute('aria-hidden', 'true');
        const text = node('span', state.label);
        text.className = browserUiClasses.chipLabel;
        chip.append(mark, text);
        const active = ACTIVE.includes(invitation.state) && invitation.expiresAtMs > now();
        const expires = node(
          'span',
          active
            ? `in ${Math.max(1, Math.ceil((invitation.expiresAtMs - now()) / 60_000))} min`
            : '—',
        );
        const actions = node('div');
        actions.className = browserUiClasses.listActions;
        actions.dataset.phone = 'menu';
        menus.push(
          mountBrowserMenu(actions, {
            label: `Actions for ${label.textContent}`,
            items: [
              ...(invitation.state === 'awaiting-owner'
                ? [{ key: 'confirm', label: 'Confirm or refuse…' }]
                : []),
              { key: 'copy', label: 'Copy invitation ID' },
              ...(ACTIVE.includes(invitation.state)
                ? [
                    {
                      key: 'cancel',
                      label: 'Cancel invitation',
                      disabled: !available(),
                      separated: true,
                    },
                  ]
                : []),
            ],
            onSelect(key) {
              if (key === 'confirm') options.confirm();
              else if (key === 'cancel') void cancel(invitation.offerId);
              else void navigator.clipboard.writeText(invitation.offerId).catch(() => {});
            },
          }),
        );
        const row = table.row([
          leading,
          title,
          chip,
          node('span', ago(invitation.issuedAtMs, now())),
          expires,
          actions,
        ]);
        row.dataset.offer = invitation.offerId;
        return row;
      }),
    );
  }
  render();
  return { render, refresh: () => refresh(null), invite, heldBy };
}
