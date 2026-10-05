import { management, reopenSession, RefusalError } from 'remote-browser-sdk';
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

function render(): void {
  element('access').textContent = {
    checking: 'Checking current access…',
    live: 'Current browser access confirmed.',
    lost: 'Current browser access refused.',
    unconfirmed: 'Current browser access unconfirmed.',
  }[page.access];
  const editable = page.access === 'live' && page.settings?.capabilities.settingsWrite === true;
  element('read-only').hidden = editable;
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
  element('outcome').textContent =
    `${page.outcome?.state ? `${page.outcome.state}: ` : ''}${page.notice || 'No change submitted.'}`;
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
        name.value = device.name;
        name.required = true;
        name.maxLength = 64;
        label.htmlFor = name.id;
        label.textContent = 'Device name';
        const save = document.createElement('button');
        save.type = 'submit';
        save.textContent = 'Rename';
        const revoke = document.createElement('button');
        revoke.type = 'button';
        revoke.textContent = 'Revoke';
        revoke.addEventListener('click', () => {
          if (!page.writable) return;
          // Confirmation is local presentation, never server authority.
          const target = page.devices?.devices.find((item) => item.clientId === device.clientId);
          if (!target || target.revoked) return;
          if (confirm(`Revoke ${target.name}${target.thisBrowser ? ' (this browser)' : ''}?`))
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
        form.append(label, name, save, revoke);
        row.append(summary, form);
        rows.set(device.clientId, row);
        devices.append(row);
      }
      row.querySelector('.device-summary')!.textContent =
        `${device.name}${device.thisBrowser ? ' · This browser' : ''} · ${device.kind} · ${device.revoked ? 'Revoked' : 'Paired'} · ${device.liveSessionCount} live sessions · Last activity ${device.lastActivityAtMs === null ? 'unavailable' : new Date(device.lastActivityAtMs).toLocaleString()}`;
      const name = row.querySelector<HTMLInputElement>('input')!;
      name.disabled = !editable || device.revoked;
      if (editable) name.removeAttribute('aria-describedby');
      else name.setAttribute('aria-describedby', 'read-only');
      for (const button of row.querySelectorAll<HTMLButtonElement>('button'))
        button.disabled = !page.writable || device.revoked;
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
async function change(intent: PageIntent): Promise<void> {
  await run(() => page.submit(intent));
  if (page.outcome?.state === 'committed' && !page.outcome.sessionEnded)
    await run(() => page.refresh());
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
refresh.addEventListener('click', () => void run(() => page.refresh()));
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
  void run(() => page.refresh(cursor));
}
more.addEventListener('click', () => navigate(page.devices?.nextCursor ?? null));
first.addEventListener('click', () => navigate(null));
recover.addEventListener(
  'click',
  () =>
    void run(async () => {
      await page.recover();
      if (page.access === 'live') await page.refresh();
    }),
);
try {
  let session = await reopenSession();
  page = new ManagementPage(management(session), async () => {
    session = await reopenSession(session);
    return management(session);
  });
  await run(() => page.refresh());
} catch (error) {
  element('access').textContent =
    error instanceof RefusalError
      ? 'Current browser access refused. Use the local CLI.'
      : 'Current browser access unconfirmed. Pair locally with tmt remote pair, or use the local CLI.';
  refresh.disabled = true;
}
