/** One retained People view; the signed member service owns admission and effects. */
import type {
  MemberChange,
  MemberMachines,
  MemberOriginalOutcome,
  MemberRoster,
  RemoteMembers,
} from './members.js';
import { memberName } from './management-codec.js';
import { readPersonDrafts, writePersonDraft } from './person-drafts.js';
import { createListTable } from '@tmt/browser-ui/list-table';
import { createIcon } from '@tmt/browser-ui/glyph';
import { mountBrowserMenu, type BrowserMenuHost } from '@tmt/browser-ui/menu-host';

export type MemberIntent =
  | { kind: 'rename'; input: MemberChange & { name: string } }
  | { kind: 'sending'; input: MemberChange & { enabled: boolean } }
  | { kind: 'revoke'; input: MemberChange };

/** Keeps one roster page and one explicitly opened machine page, never a second authority. */
export class MembersPage {
  roster?: MemberRoster;
  machines?: MemberMachines;
  intent?: Readonly<MemberIntent>;
  outcome?: MemberOriginalOutcome;
  access: 'checking' | 'live' | 'lost' | 'unconfirmed' = 'checking';
  machineAccess: 'idle' | 'checking' | 'confirmed' | 'unconfirmed' = 'idle';
  busy = false;
  private cursor: string | null = null;
  private generation = 0;
  private disposed = false;

  constructor(
    private client: RemoteMembers,
    private readonly reopen: () => Promise<RemoteMembers>,
    private readonly shouldReopen: (error: unknown) => boolean = () => false,
    private readonly endedAccess: (error: unknown) => boolean = () => false,
  ) {}

  get onFirstPage(): boolean {
    return this.cursor === null;
  }

  get canRecover(): boolean {
    return !!this.intent && this.outcome?.state === 'unknown' && !this.busy;
  }

  /** Refresh the one visible roster; an access failure must not leave old facts on screen. */
  async refresh(cursor: string | null = this.cursor): Promise<void> {
    if (this.busy || this.disposed) return;
    this.busy = true;
    const turn = ++this.generation;
    try {
      let roster: MemberRoster;
      try {
        roster = await this.client.list({ cursor, limit: 25 });
      } catch (error) {
        if (!this.shouldReopen(error)) throw error;
        const client = await this.reopen();
        if (!this.current(turn)) return;
        this.client = client;
        roster = await client.list({ cursor, limit: 25 });
      }
      if (!this.current(turn)) return;
      this.roster = roster;
      this.cursor = cursor;
      // Machine observations belong to the preceding acquisition; expand again for fresh facts.
      this.machines = undefined;
      this.machineAccess = 'idle';
      this.access = 'live';
    } catch (error) {
      if (!this.current(turn)) return;
      this.clearAccess(this.endedAccess(error) ? 'lost' : 'unconfirmed');
    } finally {
      if (this.current(turn)) this.busy = false;
    }
  }

  /** Machine reads happen only on explicit expansion or paging for an admitted member. */
  async openMachines(memberId: string, cursor: string | null = null): Promise<void> {
    if (
      this.busy ||
      this.disposed ||
      this.access !== 'live' ||
      !this.roster?.members.some((member) => member.memberId === memberId)
    )
      return;
    this.busy = true;
    this.machines = undefined;
    this.machineAccess = 'checking';
    const turn = ++this.generation;
    try {
      const machines = await this.client.machines({ memberId, cursor, limit: 25 });
      if (this.current(turn)) {
        this.machines = machines;
        this.machineAccess = 'confirmed';
      }
    } catch (error) {
      if (this.current(turn)) {
        this.machines = undefined;
        this.machineAccess = 'unconfirmed';
        if (this.endedAccess(error)) this.clearAccess('lost');
      }
    } finally {
      if (this.current(turn)) this.busy = false;
    }
  }

  /** Freeze the original before one effect; uncertain effects are never resubmitted. */
  async submit(intent: MemberIntent): Promise<void> {
    if (this.busy || this.disposed || this.access !== 'live' || this.outcome?.state === 'unknown')
      throw new Error('People controls are unavailable.');
    const target = this.roster?.members.find((member) => member.memberId === intent.input.memberId);
    if (
      !target ||
      target.remoteId !== intent.input.remoteId ||
      target.revision !== intent.input.memberRevision ||
      target.revokedAtMs !== null ||
      (intent.kind !== 'rename' && !this.roster?.administrator)
    )
      throw new Error('Review the current member before making this change.');
    this.intent = Object.freeze({
      ...intent,
      input: Object.freeze({ ...intent.input }),
    }) as MemberIntent;
    this.outcome = undefined;
    this.busy = true;
    const turn = ++this.generation;
    try {
      const original = this.intent;
      const outcome =
        original.kind === 'rename'
          ? await this.client.rename(original.input)
          : original.kind === 'sending'
            ? await this.client.sending(original.input)
            : await this.client.revoke(original.input);
      if (!this.current(turn)) return;
      this.outcome = outcome;
      if (outcome.state === 'committed' && outcome.sessionEnded) this.clearAccess('lost');
      if (outcome.state === 'refused' && outcome.reason === 'REMOTE_MANAGEMENT_READ_ONLY')
        this.clearAccess();
    } catch {
      if (this.current(turn)) this.outcome = this.unknown();
    } finally {
      if (this.current(turn)) this.busy = false;
    }
  }

  /** Each explicit check reopens and reads only the frozen original; never repeat its effect. */
  async recover(): Promise<void> {
    if (!this.canRecover || this.disposed) return;
    this.busy = true;
    const turn = ++this.generation;
    try {
      const client = await this.reopen();
      if (!this.current(turn)) return;
      this.client = client;
      const outcome = await client.operation(this.intent!.input.operationId);
      if (!this.current(turn)) return;
      this.outcome = this.matches(outcome) ? outcome : this.unknown();
      if (this.outcome.state === 'committed' && this.outcome.sessionEnded) this.clearAccess('lost');
    } catch {
      if (this.current(turn)) this.clearAccess();
    } finally {
      if (this.current(turn)) this.busy = false;
    }
  }

  /** A lost shared Session invalidates facts without discarding an original effect. */
  invalidateAccess(ended = false): void {
    if (this.disposed) return;
    if (this.busy && this.intent && !this.outcome) this.outcome = this.unknown();
    this.generation++;
    this.busy = false;
    this.clearAccess(ended ? 'lost' : 'unconfirmed');
  }

  dispose(): void {
    this.disposed = true;
    this.generation++;
    this.roster = undefined;
    this.machines = undefined;
  }

  private clearAccess(access: 'lost' | 'unconfirmed' = 'unconfirmed'): void {
    this.roster = undefined;
    this.machines = undefined;
    this.machineAccess = 'idle';
    this.access = access;
  }

  private current(turn: number): boolean {
    return !this.disposed && this.generation === turn;
  }

  private unknown(): MemberOriginalOutcome {
    return {
      operationId: this.intent!.input.operationId,
      state: 'unknown',
      reason: 'effect_outcome_unconfirmed',
    };
  }

  private matches(outcome: MemberOriginalOutcome): boolean {
    const intent = this.intent!;
    if (outcome.operationId !== intent.input.operationId) return false;
    if (outcome.state !== 'committed') return true;
    if (!('member' in outcome.result)) return false;
    const member = outcome.result.member;
    if (
      member.remoteId !== intent.input.remoteId ||
      member.memberId !== intent.input.memberId ||
      member.revision < intent.input.memberRevision
    )
      return false;
    return intent.kind === 'rename'
      ? member.displayName === intent.input.name
      : intent.kind === 'sending'
        ? member.sendingEnabled === intent.input.enabled
        : member.revokedAtMs !== null && !member.sendingEnabled;
  }
}

/** Optional pending/unknown observations; no recorded work is not a cleanup success. */
export function pendingMemberObservations(view: MemberRoster | undefined): string {
  if (!view) return '';
  const notices: string[] = [];
  if (view.invitation?.state === 'current' && view.invitation.cleanup === 'unconfirmed')
    notices.push('Invitation cleanup is unconfirmed.');
  if (view.routes.state === 'unavailable') notices.push('Access update status is unavailable.');
  if (view.routes.state === 'recorded') {
    if (view.routes.publicationPending) notices.push('An access update is awaiting delivery.');
    if (view.routes.removalPending) notices.push('An access removal is awaiting cleanup.');
  }
  return notices.join(' ');
}

/** Mount the People presentation around the retained member view and its original-outcome owner. */
export function mountMembersPage(
  root: HTMLElement,
  page: MembersPage,
  signal: AbortSignal,
  otherBusy: () => boolean,
  changed: () => void = () => {},
  observations?: HTMLElement | null,
): { render: () => void; refresh: () => Promise<void>; heldBy: () => string } {
  const node = <K extends keyof HTMLElementTagNameMap>(tag: K, text = '') => {
    const value = document.createElement(tag);
    value.textContent = text;
    return value;
  };
  const button = (text: string) => {
    const value = node('button', text);
    value.type = 'button';
    value.className = 'tmt-ui-action';
    return value;
  };
  const status = node('p', 'Checking people…');
  status.setAttribute('role', 'status');
  const facts = node('p');
  const table = createListTable(
    [
      { id: 'icon', label: '', track: '48px', hideBelow: 'phone' },
      { id: 'person', label: 'Person', track: 'minmax(0, 2.2fr)' },
      { id: 'role', label: 'Role', track: 'minmax(0, .9fr)' },
      { id: 'sending', label: 'Sending', track: 'minmax(0, 1fr)' },
      { id: 'machines', label: 'Machines', track: 'minmax(0, 1.2fr)' },
      { id: 'actions', label: '', track: '36px' },
    ],
    'People',
  );
  const list = table.body;
  const footer = node('p');
  footer.className = 'people-list-footer';
  const refreshButton = button('Refresh people');
  const first = button('First people');
  const more = button('Next people');
  const original = node('p');
  const recover = button('Check original outcome');
  const rows = new Map<
    string,
    {
      root: HTMLElement;
      summary: HTMLElement;
      nameText: HTMLElement;
      roleText: HTMLElement;
      roleChip: HTMLElement;
      sendingText: HTMLElement;
      sendingChip: HTMLElement;
      nameActions: HTMLElement;
      memberActions: HTMLElement;
      menu: BrowserMenuHost<string>;
      input: HTMLInputElement;
      baseline: string;
      save: HTMLButtonElement;
      restore: HTMLButtonElement;
      send: HTMLButtonElement;
      revoke: HTMLButtonElement;
      expand: HTMLButtonElement;
      machines: HTMLElement;
      machineKey: string;
      confirmation: HTMLElement;
      cancel: HTMLButtonElement;
    }
  >();
  let localNotice = '';
  let draftsLoaded = false;
  function persistDraft(memberId: string, baseline: string, value: string): void {
    const remoteId = page.roster?.remoteId;
    if (!remoteId) return;
    try {
      writePersonDraft(
        sessionStorage,
        remoteId,
        memberId,
        value === baseline ? undefined : { baseline, value },
      );
    } catch {
      localNotice = 'This browser could not retain the unsent name after reload.';
    }
  }
  let selected: string | undefined;
  let machineCursor: string | null = null;
  let confirmation:
    | { memberId: string; kind: 'sending' | 'revoke'; revision: number; enabled?: boolean }
    | undefined;
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

  function draftReason(): string {
    for (const row of rows.values())
      if (row.input.value !== row.baseline) return 'Save or restore the unsent person name first.';
    return '';
  }
  function heldBy(): string {
    if (page.busy) return 'A request is in progress. Wait for its outcome.';
    if (page.outcome?.state === 'unknown')
      return 'A change is unconfirmed. Check its original outcome first.';
    return draftReason();
  }
  function available(): boolean {
    return (
      page.access === 'live' && !page.busy && !otherBusy() && page.outcome?.state !== 'unknown'
    );
  }
  async function run(action: () => Promise<void>): Promise<boolean> {
    localNotice = '';
    let started = true;
    const pending = action();
    render();
    changed();
    try {
      await pending;
    } catch {
      started = false;
      localNotice = 'This action could not start. Review current access and try again.';
    }
    if (!signal.aborted) {
      render();
      changed();
    }
    return started;
  }
  async function refresh(): Promise<void> {
    if (page.busy || otherBusy()) return;
    if (
      page.roster &&
      [...rows].some(
        ([id, row]) =>
          !page.roster!.members.some((member) => member.memberId === id) &&
          row.input.value !== row.baseline,
      )
    ) {
      localNotice = 'Restore the name for the person no longer shown before refreshing.';
      render();
      return;
    }
    confirmation = undefined;
    selected = undefined;
    machineCursor = null;
    await run(() => page.refresh());
  }
  async function change(intent: MemberIntent): Promise<void> {
    confirmation = undefined;
    const submitted = await run(() => page.submit(intent));
    if (submitted && page.outcome?.state === 'committed' && !page.outcome.sessionEnded)
      await run(() => page.refresh());
  }
  function confirm(memberId: string, kind: 'sending' | 'revoke'): void {
    const member = page.roster?.members.find((item) => item.memberId === memberId);
    if (
      !member ||
      member.role === 'owner' ||
      !page.roster?.administrator ||
      !available() ||
      member.revokedAtMs !== null
    )
      return;
    const enabled = kind === 'sending' ? !member.sendingEnabled : undefined;
    if (
      confirmation?.memberId === memberId &&
      confirmation.kind === kind &&
      confirmation.revision === member.revision &&
      confirmation.enabled === enabled
    ) {
      const input = {
        operationId: crypto.randomUUID(),
        remoteId: member.remoteId,
        memberId,
        memberRevision: member.revision,
      };
      void change(
        kind === 'sending' ? { kind, input: { ...input, enabled: enabled! } } : { kind, input },
      );
      return;
    }
    confirmation = { memberId, kind, revision: member.revision, enabled };
    render();
  }
  function rowFor(memberId: string, name: string) {
    const item = node('article');
    item.className = 'people-person';
    const summary = node('p');
    summary.className = 'tmt-ui-visually-hidden';
    const nameText = node('span', name);
    const person = node('span');
    person.className = 'tmt-ui-list-leading';
    person.append(createIcon('user'));
    const roleText = node('span');
    roleText.className = 'tmt-ui-chip-label';
    const roleChip = node('span');
    roleChip.className = 'tmt-ui-chip remote-role';
    roleChip.append(roleText);
    const sendingText = node('span');
    sendingText.className = 'tmt-ui-chip-label';
    const sendingChip = node('span');
    sendingChip.className = 'tmt-ui-chip remote-stage';
    const sendingMark = node('span');
    sendingMark.className = 'tmt-ui-state-mark';
    sendingMark.dataset.shape = 'dot';
    sendingMark.setAttribute('aria-hidden', 'true');
    sendingChip.append(sendingMark, sendingText);
    const field = node('div');
    field.className = 'tmt-ui-field';
    const label = node('label', 'Person name');
    label.className = 'tmt-ui-field-label';
    const input = node('input');
    input.id = `people-name-${memberId}`;
    label.htmlFor = input.id;
    input.className = 'tmt-ui-field-control';
    input.value = name;
    input.required = true;
    input.maxLength = 64;
    field.append(label, input);
    const nameActions = node('div');
    nameActions.className = 'people-name-actions';
    const memberActions = node('div');
    memberActions.className = 'people-member-actions';
    nameActions.hidden = true;
    memberActions.hidden = true;
    const save = button('Save name');
    const restore = button('Restore name');
    const send = button('Enable sending');
    const revoke = button('Remove person');
    revoke.dataset.variant = 'destructive';
    const expand = button('Show machines');
    expand.dataset.variant = 'ghost';
    const machines = node('div');
    const confirmText = node('p');
    confirmText.setAttribute('role', 'status');
    const cancel = button('Cancel');
    const menuHost = node('div');
    const menuOptions = () => ({
      label: `Actions for ${nameText.textContent}`,
      items: [
        { key: 'rename', label: 'Rename' },
        { key: 'sending', label: send.textContent ?? 'Enable sending', disabled: send.disabled },
        { key: 'revoke', label: 'Remove person', disabled: revoke.disabled, separated: true },
      ],
      onSelect(key: string) {
        if (key === 'rename') {
          nameActions.hidden = false;
          input.focus();
        } else if (key === 'sending' || key === 'revoke') {
          memberActions.hidden = false;
          confirm(memberId, key);
        }
      },
    });
    const menu = mountBrowserMenu(menuHost, menuOptions());
    const row = {
      root: item,
      summary,
      nameText,
      roleText,
      roleChip,
      sendingText,
      sendingChip,
      nameActions,
      memberActions,
      menu,
      input,
      baseline: name,
      save,
      restore,
      send,
      revoke,
      expand,
      machines,
      machineKey: '',
      confirmation: confirmText,
      cancel,
    };
    nameActions.append(field, save, restore);
    memberActions.append(send, revoke);
    const tableRow = table.row([person, nameText, roleChip, sendingChip, expand, menuHost]);
    item.append(tableRow, summary, nameActions, memberActions, machines, confirmText, cancel);
    restore.addEventListener(
      'click',
      () => {
        input.value = row.baseline;
        persistDraft(memberId, row.baseline, input.value);
        render();
      },
      { signal },
    );
    input.addEventListener(
      'input',
      () => {
        persistDraft(memberId, row.baseline, input.value);
        render();
      },
      { signal },
    );
    save.addEventListener(
      'click',
      () => {
        const member = page.roster?.members.find((value) => value.memberId === memberId);
        if (!member || !available() || input.value === row.baseline) return;
        if (!memberName(input.value)) {
          localNotice = 'Use a nonblank name of at most 64 UTF-8 bytes without control characters.';
          render();
          return;
        }
        void change({
          kind: 'rename',
          input: {
            operationId: crypto.randomUUID(),
            remoteId: member.remoteId,
            memberId,
            memberRevision: member.revision,
            name: input.value,
          },
        });
      },
      { signal },
    );
    send.addEventListener(
      'click',
      (event) => {
        if (event.detail <= 1) confirm(memberId, 'sending');
      },
      { signal },
    );
    revoke.addEventListener(
      'click',
      (event) => {
        if (event.detail <= 1) confirm(memberId, 'revoke');
      },
      { signal },
    );
    cancel.addEventListener(
      'click',
      () => {
        confirmation = undefined;
        render();
      },
      { signal },
    );
    expand.addEventListener(
      'click',
      () => {
        if (!available()) return;
        selected = memberId;
        machineCursor = null;
        void run(() => page.openMachines(memberId));
      },
      { signal },
    );
    for (const action of [send, revoke])
      action.addEventListener(
        'keydown',
        (event) => {
          if ((event.repeat || event.isComposing) && ['Enter', ' '].includes(event.key))
            event.preventDefault();
          if (event.key === 'Escape') {
            confirmation = undefined;
            render();
          }
        },
        { signal },
      );
    rows.set(memberId, row);
    list.append(item);
    return row;
  }
  function render(): void {
    if (signal.aborted) return;
    const view = page.roster;
    footer.replaceChildren(
      node(
        'span',
        view
          ? `${view.members.length} ${view.members.length === 1 ? 'person' : 'people'} in this page`
          : '',
      ),
      node('span', 'Remove person · Enable / Disable sending in the row menu'),
    );
    footer.hidden = !view;
    const outcome = page.outcome;
    if (localNotice) status.textContent = localNotice;
    else if (outcome?.state === 'unknown')
      status.textContent =
        'This change is unconfirmed. Check its original outcome before another change.';
    else if (outcome?.state === 'refused')
      status.textContent =
        'The change was refused. Review current access or use the local terminal.';
    else if (outcome?.state === 'committed')
      status.textContent = outcome.sessionEnded
        ? 'The change was saved and this session ended.'
        : 'The change was saved.';
    else
      status.textContent =
        page.access === 'live'
          ? 'People visible to this browser.'
          : page.access === 'checking'
            ? 'Checking people…'
            : page.access === 'lost'
              ? 'This browser’s access ended. Use tmt remote members on this machine.'
              : 'People could not be confirmed. Use tmt remote members on this machine.';
    original.textContent =
      outcome?.state === 'unknown' && page.intent
        ? `Original change ${page.intent.input.operationId}`
        : '';
    original.hidden = !original.textContent;
    recover.hidden = outcome?.state !== 'unknown';
    recover.disabled = !page.canRecover || otherBusy();
    refreshButton.disabled = page.busy || otherBusy();
    first.hidden = page.onFirstPage;
    more.hidden = !view?.nextCursor;
    first.disabled = more.disabled = !available() || !!draftReason();
    facts.textContent = !view
      ? ''
      : [
          view.members.length ? '' : 'No people in this page.',
          view.administrator
            ? ''
            : 'Only a designated admin can change sending or remove people. Use tmt remote members on this machine.',
        ]
          .filter(Boolean)
          .join(' ');
    facts.hidden = !facts.textContent;
    if (observations) {
      observations.textContent = pendingMemberObservations(view);
      observations.hidden = !observations.textContent;
    }
    if (view && !draftsLoaded) {
      draftsLoaded = true;
      try {
        for (const [id, draft] of readPersonDrafts(sessionStorage, view.remoteId)) {
          const member = view.members.find((item) => item.memberId === id);
          const row = rows.get(id) ?? rowFor(id, member?.displayName ?? draft.baseline);
          row.input.value = draft.value;
          if (row.input.value !== row.baseline) row.nameActions.hidden = false;
          else persistDraft(id, row.baseline, row.input.value);
        }
      } catch {
        localNotice = 'This browser could not restore an unsent name after reload.';
      }
    }
    const current = new Set(view?.members.map((member) => member.memberId) ?? []);
    for (const [id, row] of rows) {
      if (!current.has(id)) {
        if (row.input.value === row.baseline) {
          row.menu.dispose();
          row.root.remove();
          rows.delete(id);
        } else {
          row.root.hidden = false;
          row.summary.className = '';
          row.summary.textContent =
            'This person is not in the current page. Your unsent name is kept; restore it to continue.';
          row.save.disabled = row.send.disabled = row.revoke.disabled = row.expand.disabled = true;
          row.restore.hidden = false;
          row.restore.disabled = page.busy || otherBusy();
        }
      }
    }
    for (const member of view?.members ?? []) {
      const row = rows.get(member.memberId) ?? rowFor(member.memberId, member.displayName);
      row.root.hidden = false;
      row.summary.className = 'tmt-ui-visually-hidden';
      if (row.input.value === row.baseline) row.input.value = member.displayName;
      row.baseline = member.displayName;
      if (row.input.value === row.baseline)
        persistDraft(member.memberId, row.baseline, row.input.value);
      else row.nameActions.hidden = false;
      row.summary.textContent = `${member.displayName} · ${member.role === 'owner' ? 'Owner' : 'Member'} · Sending ${member.sendingEnabled ? 'on' : 'off'}${member.revokedAtMs === null ? '' : ' · Revoked'}`;
      row.nameText.textContent = member.displayName;
      row.roleText.textContent = member.role === 'owner' ? 'Owner' : 'Member';
      row.roleChip.dataset.roleTone = member.role === 'owner' ? 'selected' : 'neutral';
      row.sendingChip.dataset.tone =
        member.revokedAtMs !== null ? 'blocked' : member.sendingEnabled ? 'working' : 'neutral';
      row.sendingText.textContent =
        member.revokedAtMs !== null
          ? 'Revoked'
          : member.sendingEnabled
            ? 'Sending on'
            : 'Sending off';
      const editable = available() && member.revokedAtMs === null;
      row.input.disabled = !editable;
      row.save.disabled = !editable || row.input.value === row.baseline;
      row.restore.hidden = row.input.value === row.baseline;
      row.restore.disabled = page.busy || otherBusy();
      row.input.setAttribute('aria-label', `Person name for ${member.displayName}`);
      row.save.setAttribute('aria-label', `Save name for ${member.displayName}`);
      row.restore.setAttribute('aria-label', `Restore name for ${member.displayName}`);
      row.send.setAttribute(
        'aria-label',
        `${member.sendingEnabled ? 'Disable' : 'Enable'} sending for ${member.displayName}`,
      );
      row.revoke.setAttribute('aria-label', `Remove ${member.displayName}`);
      row.expand.setAttribute('aria-label', `Show machines for ${member.displayName}`);
      row.send.hidden = row.revoke.hidden = member.role === 'owner';
      row.send.disabled = row.revoke.disabled =
        member.role === 'owner' || !editable || !view!.administrator;
      row.send.textContent = member.sendingEnabled ? 'Disable sending' : 'Enable sending';
      row.expand.disabled = !available();
      row.expand.textContent = selected === member.memberId ? 'Refresh machines' : 'Show machines';
      const machineKey = JSON.stringify([
        selected === member.memberId,
        page.machineAccess,
        page.machines,
        machineCursor,
        available(),
        member.displayName,
      ]);
      if (row.machineKey !== machineKey) {
        row.machineKey = machineKey;
        row.machines.replaceChildren();
        if (selected === member.memberId && page.machineAccess === 'checking')
          row.machines.append(node('p', 'Checking machines…'));
        if (selected === member.memberId && page.machineAccess === 'unconfirmed')
          row.machines.append(node('p', 'Machine observations could not be confirmed.'));
        if (selected === member.memberId && page.machines?.memberId === member.memberId) {
          if (!page.machines.machines.length)
            row.machines.append(node('p', 'No machines in this page.'));
          for (const machine of page.machines.machines) {
            const observed =
              machine.liveSessionCount === null
                ? 'Activity unavailable'
                : machine.liveSessionCount === 0
                  ? 'No live sessions reported'
                  : `${machine.liveSessionCount} live ${machine.liveSessionCount === 1 ? 'session' : 'sessions'} reported`;
            row.machines.append(
              node(
                'p',
                `${machine.name} · ${machine.kind}${machine.thisBrowser ? ' · This browser' : ''}${machine.revoked ? ' · Revoked' : machine.expiresAtMs !== null && machine.expiresAtMs <= Date.now() ? ' · Expiry passed locally' : machine.grantLive ? ' · Access eligible' : ' · Access not active'} · ${observed} · Last used ${machine.lastUsedAtMs === null ? 'not recorded' : new Date(machine.lastUsedAtMs).toLocaleString()} · Last activity ${machine.lastActivityAtMs === null ? 'not observed' : new Date(machine.lastActivityAtMs).toLocaleString()}`,
              ),
            );
          }
          if (machineCursor !== null) {
            const firstMachine = button('First machines');
            firstMachine.setAttribute('aria-label', `First machines for ${member.displayName}`);
            firstMachine.disabled = !available();
            firstMachine.addEventListener(
              'click',
              () => {
                machineCursor = null;
                void run(() => page.openMachines(member.memberId));
              },
              { signal },
            );
            row.machines.append(firstMachine);
          }
          if (page.machines.nextCursor) {
            const next = button('Next machines');
            next.setAttribute('aria-label', `Next machines for ${member.displayName}`);
            next.disabled = !available();
            next.addEventListener(
              'click',
              () => {
                const cursor = page.machines?.nextCursor;
                if (cursor) {
                  machineCursor = cursor;
                  void run(() => page.openMachines(member.memberId, cursor));
                }
              },
              { signal },
            );
            row.machines.append(next);
          }
        }
      }
      const pending =
        confirmation?.memberId === member.memberId && confirmation.revision === member.revision;
      row.confirmation.textContent = !pending
        ? ''
        : confirmation!.kind === 'revoke'
          ? `This removes ${member.displayName}'s Remote access. Press Remove person again to confirm.`
          : `${confirmation!.enabled ? 'Enable' : 'Disable'} sending for ${member.displayName}. Press the same action again to confirm.`;
      row.confirmation.hidden = !pending;
      row.cancel.hidden = !pending;
      row.memberActions.hidden = !pending;
      row.menu.update({
        label: `Actions for ${member.displayName}`,
        items: [
          { key: 'rename', label: 'Rename', disabled: row.input.disabled },
          {
            key: 'sending',
            label: row.send.textContent ?? 'Enable sending',
            disabled: row.send.disabled,
          },
          { key: 'revoke', label: 'Remove person', disabled: row.revoke.disabled, separated: true },
        ],
        onSelect(key) {
          if (key === 'rename') {
            row.nameActions.hidden = false;
            row.input.focus();
          } else if (key === 'sending' || key === 'revoke') {
            row.memberActions.hidden = false;
            confirm(member.memberId, key);
          }
        },
      });
    }
  }
  refreshButton.addEventListener('click', () => void refresh(), { signal });
  first.addEventListener(
    'click',
    () => {
      if (!heldBy()) void run(() => page.refresh(null));
    },
    { signal },
  );
  more.addEventListener(
    'click',
    () => {
      const cursor = page.roster?.nextCursor;
      if (cursor && !heldBy()) void run(() => page.refresh(cursor));
    },
    { signal },
  );
  recover.addEventListener('click', () => void run(() => page.recover()), { signal });
  signal.addEventListener(
    'abort',
    () => {
      for (const row of rows.values()) row.menu.dispose();
      page.dispose();
    },
    { once: true },
  );
  render();
  return { render, refresh, heldBy };
}
