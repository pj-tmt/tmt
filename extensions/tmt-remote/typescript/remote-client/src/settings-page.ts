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
const recover = element<HTMLButtonElement>('recover');
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
  element('outcome-notice').dataset.tone = page.busy
    ? 'waiting'
    : page.outcome?.state === 'committed'
      ? 'working'
      : page.outcome?.state === 'unknown' || page.outcome?.state === 'refused'
        ? 'blocked'
        : 'review';
  const reason = page.busy
    ? 'A request is in progress. Wait for its outcome.'
    : !editable
      ? 'Changes are unavailable in this browser. Use the local CLI.'
      : page.outcome?.state === 'unknown'
        ? 'The original outcome is unknown. Read it before another change.'
        : page.outcome?.state === 'refused' && page.outcome.reason === 'REMOTE_MANAGEMENT_CAPACITY'
          ? 'Browser management operation limit reached. Use the local CLI; do not retry or reset storage.'
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
  refresh.disabled = page.busy;
  recover.hidden = !page.canRecover;
  more.hidden = !page.devices?.nextCursor;
  more.disabled = page.busy;
  first.hidden = page.onFirstPage;
  first.disabled = page.busy;
  commandNotice(
    element('outcome'),
    `${page.outcome?.state && page.outcome.state !== 'unknown' ? `${page.outcome.state}: ` : ''}${page.notice || 'No change submitted.'}`,
  );
  element('original').textContent = page.intent
    ? `Original operation ${page.intent.input.operationId}`
    : '';
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
        row.append(summary, form, disabledReason);
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
      disabledReason.textContent = device.revoked ? 'This device is revoked.' : reason;
      disabledReason.hidden = !disabledReason.textContent;
      for (const button of row.querySelectorAll<HTMLButtonElement>('button')) {
        button.disabled = !page.writable || device.revoked;
        if (disabledReason.textContent) button.setAttribute('aria-describedby', disabledReason.id);
        else button.removeAttribute('aria-describedby');
        button.setAttribute('aria-busy', String(page.busy));
      }
    }
  }
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
  if ((opening.value === 'on') === page.settings?.settings.open) return;
  void change({
    kind: 'setting',
    input: { operationId: crypto.randomUUID(), setting: 'open', value: opening.value === 'on' },
  });
});
element<HTMLFormElement>('limit-form').addEventListener('submit', (event) => {
  event.preventDefault();
  if (!page.writable) return;
  // An untouched unset/default projection is preserved, not silently saved as explicit 8.
  if (mode.value === 'default') return;
  const value = mode.value === 'off' ? null : custom.value;
  if (
    value === page.settings?.settings.sessionsPerDevice &&
    page.settings.settings.sessionsPerDeviceSource === 'settings.json'
  )
    return;
  void change({
    kind: 'setting',
    input: { operationId: crypto.randomUUID(), setting: 'sessions-per-device', value },
  });
});
mode.addEventListener('change', render);
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
recover.addEventListener(
  'click',
  () =>
    void run(async () => {
      await page.recover();
      if (page.access === 'live') await refreshView();
    }),
);
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
