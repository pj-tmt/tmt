import { management, reopenSession, RefusalError } from 'remote-browser-sdk';
import { renderFirestore } from './firestore-page.js';
import { ManagementPage, type PageIntent } from './management-page.js';

function element<T extends HTMLElement>(id: string): T {
  const found = document.getElementById(id);
  if (!found) throw new Error(`Settings page lacks #${id}.`);
  return found as T;
}
const opening = element<HTMLSelectElement>('opening');
const mode = element<HTMLSelectElement>('limit-mode');
const custom = element<HTMLInputElement>('limit-custom');
const devices = element<HTMLDivElement>('devices');
const refresh = element<HTMLButtonElement>('refresh');
const more = element<HTMLButtonElement>('more');
const first = element<HTMLButtonElement>('first');
const rows = new Map<string, HTMLElement>();
let initialized = false;
let page: ManagementPage;

/** Format fixed CLI hints as literal text, never interpret notice content as HTML. */
function commandNotice(target: HTMLElement, text: string): void {
  target.replaceChildren();
  for (const part of text.split(/(tmt remote (?:pair|devices|settings))/g)) {
    if (/^tmt remote (?:pair|devices|settings)$/.test(part)) {
      const code = document.createElement('code');
      code.className = 'tmt-ui-code';
      code.textContent = part;
      target.append(code);
    } else target.append(document.createTextNode(part));
  }
}

function openingChanged(): boolean {
  return !!page.settings && (opening.value === 'on') !== page.settings.settings.open;
}
function limitChanged(): boolean {
  if (!page.settings || mode.value === 'default') return false;
  const admitted = page.settings.settings;
  const value = mode.value === 'off' ? null : custom.value;
  return (
    admitted.sessionsPerDeviceSource !== 'settings.json' || value !== admitted.sessionsPerDevice
  );
}
function saveState(form: string, hint: string, changed: boolean, valid: boolean): void {
  const button =
    element<HTMLFormElement>(form).querySelector<HTMLButtonElement>('button[type=submit]')!;
  button.disabled = !page.writable || !changed || !valid;
  element(hint).hidden = !page.writable || changed;
  const described = button.getAttribute('aria-describedby');
  if (!element(hint).hidden)
    button.setAttribute('aria-describedby', [described, hint].filter(Boolean).join(' '));
}
function recoverOriginal(): void {
  void run(async () => {
    await page.recover();
    if (page.access === 'live') await refreshView();
  });
}
/** Mount an empty live region before a device can originate a change. Never move it. */
function deviceFeedback(clientId: string): HTMLElement {
  const feedback = document.createElement('div');
  feedback.className = 'change-feedback';
  feedback.dataset.feedback = clientId;
  const status = document.createElement('p');
  status.dataset.outcomeSlot = '';
  status.setAttribute('role', 'status');
  status.setAttribute('aria-live', 'polite');
  status.setAttribute('aria-atomic', 'true');
  const original = document.createElement('p');
  original.dataset.original = '';
  original.hidden = true;
  const recover = document.createElement('button');
  recover.type = 'button';
  recover.className = 'tmt-ui-action';
  recover.dataset.recover = '';
  recover.hidden = true;
  const label = document.createElement('span');
  label.className = 'tmt-ui-action-label';
  label.textContent = 'Check original result';
  recover.append(label);
  recover.addEventListener('click', recoverOriginal);
  feedback.append(status, original, recover);
  return feedback;
}
function renderOutcome(): void {
  const origin =
    page.intent?.kind === 'setting' ? page.intent.input.setting : page.intent?.input.clientId;
  const target =
    origin &&
    [...document.querySelectorAll<HTMLElement>('[data-feedback]')].find(
      (slot) => slot.dataset.feedback === origin,
    );
  const active = target || document.querySelector<HTMLElement>('[data-feedback="shared"]')!;
  // Clear the former projection before publishing into another pre-existing slot.
  for (const slot of document.querySelectorAll<HTMLElement>('[data-feedback]')) {
    if (slot !== active) {
      const status = slot.querySelector<HTMLElement>('[data-outcome-slot]')!;
      if (status.textContent) commandNotice(status, '');
    }
  }
  // IDs select the single active projection for existing observers; the role/status
  // elements themselves were mounted empty and stay attached to their original forms.
  for (const slot of document.querySelectorAll<HTMLElement>('[data-feedback]')) {
    const status = slot.querySelector<HTMLElement>('[data-outcome-slot]')!;
    const original = slot.querySelector<HTMLElement>('[data-original]')!;
    const recover = slot.querySelector<HTMLButtonElement>('[data-recover]')!;
    const selected = slot === active;
    status.id = selected ? 'outcome' : '';
    original.id = selected ? 'original' : '';
    recover.id = selected ? 'recover' : '';
    // Avoid clearing/rewriting the populated slot on unrelated renders: no repeat announcement.
    const text = selected ? page.notice : '';
    if (status.textContent !== text) commandNotice(status, text);
    status.dataset.state = selected ? (page.outcome?.state ?? 'pending') : '';
    slot.dataset.tone = page.busy
      ? 'waiting'
      : page.outcome?.state === 'committed'
        ? 'working'
        : 'blocked';
    original.textContent =
      selected &&
      page.intent &&
      (page.outcome?.state === 'unknown' || page.outcome?.state === 'refused')
        ? `Original operation ${page.intent.input.operationId}`
        : '';
    original.hidden = !original.textContent;
    recover.hidden = !selected || !page.canRecover;
    recover.disabled = page.busy;
    recover.setAttribute('aria-busy', String(page.busy));
  }
}

function render(): void {
  renderFirestore(element('firestore-content'), page.firestoreAccess, page.firestore);
  element('access').textContent = {
    checking: 'Checking current access…',
    live: 'Current browser access confirmed.',
    lost: 'Current browser access refused.',
    unconfirmed: 'Current browser access unconfirmed.',
  }[page.access];
  const editable = page.access === 'live' && page.settings?.capabilities.settingsWrite === true;
  element('read-only').hidden = editable;
  element('access-notice').dataset.tone =
    page.access === 'live'
      ? editable
        ? 'working'
        : 'review'
      : page.access === 'checking'
        ? 'waiting'
        : 'blocked';
  const reason = page.busy
    ? 'A request is in progress. Wait for its outcome.'
    : !editable
      ? 'Changes are unavailable in this browser. Use the local CLI.'
      : page.outcome?.state === 'unknown'
        ? 'Another change is still unconfirmed. Check its original result first.'
        : page.outcome?.state === 'refused' && page.outcome.reason === 'REMOTE_MANAGEMENT_CAPACITY'
          ? 'Browser change limit reached. Use the local CLI; do not retry or reset storage.'
          : '';
  element('controls-reason').textContent = reason;
  element('controls-reason').hidden = !reason;
  for (const button of document.querySelectorAll<HTMLButtonElement>('button')) {
    if (reason) button.setAttribute('aria-describedby', 'controls-reason');
    else button.removeAttribute('aria-describedby');
    button.setAttribute('aria-busy', String(page.busy));
  }

  const openingForm = element<HTMLFormElement>('opening-form');
  if (editable) openingForm.removeAttribute('aria-describedby');
  else openingForm.setAttribute('aria-describedby', 'read-only');
  element('limit-form').setAttribute(
    'aria-describedby',
    editable ? 'limit-help' : 'limit-help read-only',
  );
  if (page.settings) {
    const value = page.settings.settings;
    element('opening-value').textContent = `${value.open ? 'On' : 'Off'} · ${value.source}`;
    element('limit-value').textContent =
      `${value.sessionsPerDevice ?? 'Off (unlimited)'} · ${value.sessionsPerDeviceSource}`;
    const warning = element('warning');
    warning.textContent = value.warning ?? '';
    warning.hidden = value.warning === null;
    if (!initialized) {
      opening.value = value.open ? 'on' : 'off';
      mode.value =
        value.sessionsPerDeviceSource === 'default'
          ? 'default'
          : value.sessionsPerDevice === null
            ? 'off'
            : 'custom';
      custom.value = value.sessionsPerDevice ?? '';
      initialized = true;
    }
  }
  custom.required = mode.value === 'custom';
  for (const field of [opening, mode, custom])
    field.disabled = !editable || (field === custom && mode.value !== 'custom');
  for (const button of document.querySelectorAll<HTMLButtonElement>('form button'))
    button.disabled = !page.writable;
  saveState('opening-form', 'opening-unchanged', openingChanged(), opening.checkValidity());
  saveState(
    'limit-form',
    'limit-unchanged',
    limitChanged(),
    mode.value !== 'custom' || custom.checkValidity(),
  );
  refresh.disabled = page.busy;
  more.hidden = !page.devices?.nextCursor;
  more.disabled = page.busy;
  first.hidden = page.onFirstPage;
  first.disabled = page.busy;
  if (page.devices) {
    const current = new Set(page.devices.devices.map((device) => device.clientId));
    for (const [id, row] of rows)
      if (!current.has(id)) {
        row.remove();
        rows.delete(id);
      }
    for (const device of page.devices.devices) {
      let row = rows.get(device.clientId);
      if (!row) {
        row = document.createElement('div');
        row.className = 'device';
        const summary = document.createElement('p');
        summary.className = 'device-summary';
        const form = document.createElement('form');
        const label = document.createElement('label');
        const name = document.createElement('input');
        name.id = `name-${device.clientId}`;
        name.className = 'tmt-ui-field-control';
        label.id = `${name.id}-label`;
        label.className = 'tmt-ui-field-label';
        name.setAttribute('aria-labelledby', label.id);
        name.value = device.name;
        name.required = true;
        name.maxLength = 64;
        label.htmlFor = name.id;
        label.textContent = 'Device name';
        const save = document.createElement('button');
        save.type = 'submit';
        save.className = 'tmt-ui-action';
        const saveLabel = document.createElement('span');
        saveLabel.className = 'tmt-ui-action-label';
        saveLabel.textContent = 'Rename';
        save.append(saveLabel);
        const revoke = document.createElement('button');
        revoke.type = 'button';
        revoke.className = 'tmt-ui-action';
        revoke.dataset.variant = 'destructive';
        const revokeLabel = document.createElement('span');
        revokeLabel.className = 'tmt-ui-action-label';
        revokeLabel.textContent = 'Revoke';
        revoke.append(revokeLabel);
        revoke.addEventListener('click', () => {
          if (!page.writable) return;
          // Confirmation is local presentation, never server authority.
          const target = page.devices?.devices.find((item) => item.clientId === device.clientId);
          if (!target || target.revoked) return;
          if (confirm(`Revoke ${target.name}${target.thisBrowser ? ' (this device)' : ''}?`))
            void change({
              kind: 'revoke',
              input: { operationId: crypto.randomUUID(), clientId: device.clientId },
            });
        });
        form.addEventListener('submit', (event) => {
          event.preventDefault();
          if (!page.writable) return;
          void change({
            kind: 'rename',
            input: {
              operationId: crypto.randomUUID(),
              clientId: device.clientId,
              name: name.value,
            },
          });
        });
        const field = document.createElement('div');
        field.className = 'tmt-ui-field';
        field.append(label, name);
        const talk = document.createElement('button');
        talk.type = 'button';
        talk.className = 'tmt-ui-action';
        talk.dataset.action = 'talk';
        const talkLabel = document.createElement('span');
        talkLabel.className = 'tmt-ui-action-label';
        talk.append(talkLabel);
        talk.addEventListener('click', () => {
          if (!page.writable) return;
          const target = page.devices?.devices.find((item) => item.clientId === device.clientId);
          if (!target || target.revoked) return;
          if (
            !target.talkEnabled &&
            !confirm(`Allow ${target.name} to send to its permitted agents?`)
          )
            return;
          void change({
            kind: 'talk',
            input: {
              operationId: crypto.randomUUID(),
              clientId: target.clientId,
              enabled: !target.talkEnabled,
            },
          });
        });
        form.append(field, save, talk, revoke);
        const disabledReason = document.createElement('p');
        disabledReason.id = `device-reason-${device.clientId}`;
        disabledReason.className = 'tmt-ui-field-description';
        row.append(summary, form, deviceFeedback(device.clientId), disabledReason);
        rows.set(device.clientId, row);
        devices.append(row);
      }
      row.querySelector('.device-summary')!.textContent =
        `${device.name}${device.thisBrowser ? ' · This device' : ''} · ${device.kind} · ${device.revoked ? 'Revoked' : `Paired · Sending ${device.talkEnabled ? 'on' : 'off'}`} · ${device.liveSessionCount} live ${device.liveSessionCount === 1 ? 'session' : 'sessions'} · Last activity ${device.lastActivityAtMs === null ? 'unavailable' : new Date(device.lastActivityAtMs).toLocaleString()}`;
      row.querySelector('[data-action="talk"] .tmt-ui-action-label')!.textContent =
        device.talkEnabled ? 'Disable sending' : 'Enable sending';
      const name = row.querySelector<HTMLInputElement>('input')!;
      name.disabled = !editable || device.revoked;
      if (editable) name.removeAttribute('aria-describedby');
      else name.setAttribute('aria-describedby', 'read-only');
      const disabledReason = element(`device-reason-${device.clientId}`);
      disabledReason.textContent =
        page.outcome?.state === 'unknown' &&
        page.intent?.kind !== 'setting' &&
        page.intent?.input.clientId === device.clientId
          ? ''
          : device.revoked
            ? 'This device is revoked.'
            : reason;
      disabledReason.hidden = !disabledReason.textContent;
      for (const button of row.querySelectorAll<HTMLButtonElement>('form button')) {
        button.disabled = !page.writable || device.revoked;
        if (disabledReason.textContent) button.setAttribute('aria-describedby', disabledReason.id);
        else button.removeAttribute('aria-describedby');
        button.setAttribute('aria-busy', String(page.busy));
      }
    }
  }
  renderOutcome();
}
async function run(action: () => Promise<void>): Promise<void> {
  const pending = action();
  render();
  try {
    await pending;
  } catch (error) {
    page.notice = error instanceof Error ? error.message : 'Action unavailable.';
  }
  render();
}
/** Paint management first; only the optional section changes when its read completes. */
async function refreshView(cursor?: string | null): Promise<void> {
  await page.refresh(cursor);
  render();
  if (page.access === 'live')
    void page
      .observeFirestore()
      .then(() =>
        renderFirestore(element('firestore-content'), page.firestoreAccess, page.firestore),
      );
}
async function change(intent: PageIntent): Promise<void> {
  await run(() => page.submit(intent));
  if (page.outcome?.state === 'committed' && !page.outcome.sessionEnded)
    await run(() => refreshView());
}
element<HTMLFormElement>('opening-form').addEventListener('submit', (event) => {
  event.preventDefault();
  if (!page.writable) return;
  if (!openingChanged()) {
    render();
    return;
  }
  void change({
    kind: 'setting',
    input: { operationId: crypto.randomUUID(), setting: 'open', value: opening.value === 'on' },
  });
});
element<HTMLFormElement>('limit-form').addEventListener('submit', (event) => {
  event.preventDefault();
  if (!page.writable) return;
  // Keep the no-op/provenance guard before generating an ID or contacting the door.
  if (!limitChanged()) {
    render();
    return;
  }
  if (mode.value === 'custom' && !custom.checkValidity()) return;
  const value = mode.value === 'off' ? null : custom.value;
  void change({
    kind: 'setting',
    input: { operationId: crypto.randomUUID(), setting: 'sessions-per-device', value },
  });
});
for (const field of [opening, mode, custom]) {
  field.addEventListener('input', render);
  field.addEventListener('change', render);
}
refresh.addEventListener('click', () => void run(() => refreshView()));
// Navigation never discards an unsent name. Keep only this bounded page's forms,
// rather than accumulating drafts or cursor history across the whole inventory.
function navigate(cursor: string | null): void {
  if (page.busy) return;
  for (const device of page.devices?.devices ?? []) {
    const name = rows.get(device.clientId)?.querySelector<HTMLInputElement>('input');
    if (name && name.value !== device.name) {
      page.notice = 'Save or restore the unsent device name before changing pages.';
      render();
      name.focus();
      return;
    }
  }
  void run(() => refreshView(cursor));
}
more.addEventListener('click', () => navigate(page.devices?.nextCursor ?? null));
first.addEventListener('click', () => navigate(null));
for (const button of document.querySelectorAll<HTMLButtonElement>('[data-recover]'))
  button.addEventListener('click', recoverOriginal);

try {
  let session = await reopenSession();
  page = new ManagementPage(management(session), async () => {
    session = await reopenSession(session);
    return management(session);
  });
  await run(() => refreshView());
} catch (error) {
  commandNotice(
    element('access'),
    error instanceof RefusalError
      ? 'Current browser access refused. Use the local CLI.'
      : 'Current browser access unconfirmed. Pair locally with tmt remote pair, or use the local CLI.',
  );
  refresh.disabled = true;
  element('access-notice').dataset.tone = 'blocked';
  element('controls-reason').textContent = 'Current access is unavailable. Use the local CLI.';
}
