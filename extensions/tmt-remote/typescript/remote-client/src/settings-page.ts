import { settingsChoice } from './settings-controls.js';
import { remoteRegistryPage } from './remote-registry-page.js';
import { management, reopenSession, RefusalError } from 'remote-browser-sdk';
import { renderFirestore } from './firestore-page.js';
import { ManagementPage, type PageIntent } from './management-page.js';

function element<T extends HTMLElement>(id: string): T {
  const found = document.getElementById(id);
  if (!found) throw new Error(`Settings page lacks #${id}.`);
  return found as T;
}
const opening = element<HTMLButtonElement>('opening');
const mode = element<HTMLButtonElement>('limit-mode');
const custom = element<HTMLInputElement>('limit-custom');
const devices = element<HTMLDivElement>('devices');
const refresh = element<HTMLButtonElement>('refresh');
const more = element<HTMLButtonElement>('more');
const first = element<HTMLButtonElement>('first');
const rows = new Map<string, HTMLElement>();
let initialized = false;
let choices: (() => void)[] | undefined;
type Device = NonNullable<ManagementPage['devices']>['devices'][number];
let confirmation: { button: HTMLButtonElement; clientId: string; signature: string } | undefined;
function targetSignature(target: Device): string {
  return JSON.stringify([target.name, target.revoked, target.talkEnabled, target.thisBrowser]);
}
function cancelConfirmation(refocus = false): void {
  if (!confirmation) return;
  const { button, clientId } = confirmation;
  const panel = rows.get(clientId)?.querySelector<HTMLElement>('[data-confirmation]');
  if (panel) panel.hidden = true;
  button.removeAttribute('aria-describedby');
  button.setAttribute('aria-expanded', 'false');
  confirmation = undefined;
  if (refocus && button.isConnected && !button.disabled) button.focus({ preventScroll: true });
}
/** A second deliberate press confirms only the same currently admitted presentation. */
function confirmDevice(
  button: HTMLButtonElement,
  target: Device,
  kind: 'talk' | 'revoke',
): boolean {
  page.removalPreview = undefined;
  const signature = targetSignature(target);
  if (confirmation?.button === button && confirmation.signature === signature) {
    cancelConfirmation();
    return true;
  }
  cancelConfirmation();
  const panel = rows.get(target.clientId)!.querySelector<HTMLElement>('[data-confirmation]')!;
  const consequence = panel.querySelector('p')!;
  consequence.textContent =
    kind === 'talk'
      ? `${target.name} will be able to send to its permitted agents. Press Enable sending again to confirm. You can disable sending later.`
      : target.thisBrowser
        ? `This ends Remote access for this browser (${target.name}). Press Remove again to confirm.`
        : `This ends Remote access for ${target.name}. Press Remove again to confirm.`;
  panel.hidden = false;
  button.setAttribute('aria-describedby', consequence.id);
  button.setAttribute('aria-expanded', 'true');
  confirmation = { button, clientId: target.clientId, signature };
  return false;
}
let page: ManagementPage;
const registryState = remoteRegistryPage(() => {
  if (!page) return '';
  if (page.busy) return 'A request is in progress. Wait for its outcome.';
  if (page.outcome?.state === 'unknown')
    return 'Another change is still unconfirmed. Check its original result first.';
  for (const device of page.devices?.devices ?? []) {
    const name = rows.get(device.clientId)?.querySelector<HTMLInputElement>('input');
    if (name && name.value !== device.name)
      return 'Save or restore the unsent device name before changing pages.';
  }
  return '';
});

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

function capacityCopy(refusal?: InstanceType<typeof RefusalError>): string | undefined {
  return refusal?.code === 'REMOTE_SESSION_LIMIT'
    ? "This browser has reached the owner's connection limit. Close another tab, or ask the owner to change the limit. Your pairing is unchanged."
    : undefined;
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
/** Busy is not disabled: only an unavailable control is disabled. A busy one keeps its look and
 * focus and is fenced by aria-disabled; every activation handler already refuses while busy. */
function fence(button: HTMLButtonElement, unavailable: boolean): void {
  button.disabled = unavailable;
  if (page.busy && !unavailable) button.setAttribute('aria-disabled', 'true');
  else button.removeAttribute('aria-disabled');
}
const BUSY_MARK_DELAY_MS = 400;
const BUSY_MARK_HOLD_MS = 400;
let pressed: HTMLElement | undefined;
type BusyMark = { owner: HTMLElement; shownAt?: number; timer?: ReturnType<typeof setTimeout> };
let busyMark: BusyMark | undefined;
document.addEventListener(
  'click',
  (event) => {
    pressed = (event.target as Element).closest<HTMLElement>('button.tmt-ui-action') ?? undefined;
  },
  true,
);
function markSlot(button: HTMLElement): HTMLElement {
  let slot = button.querySelector<HTMLElement>(':scope > .tmt-ui-action-mark');
  if (slot) return slot;
  slot = document.createElement('span');
  slot.className = 'tmt-ui-action-mark';
  slot.setAttribute('aria-hidden', 'true');
  slot.dataset.busy = 'false';
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  for (const [name, value] of Object.entries({
    width: '1em',
    height: '1em',
    viewBox: '0 0 24 24',
    fill: 'none',
    stroke: 'currentColor',
    'stroke-width': '2',
    'stroke-linecap': 'round',
    'stroke-linejoin': 'round',
  }))
    svg.setAttribute(name, value);
  const arc = document.createElementNS('http://www.w3.org/2000/svg', 'path');
  arc.setAttribute('d', 'M21 12a9 9 0 1 1-6.219-8.56');
  svg.append(arc);
  slot.append(svg);
  button.prepend(slot);
  return slot;
}
/** The pressed action's reserved mark shows after 400 ms of busy and stays at least 400 ms. A
 * write that ends sooner changes nothing visible. */
function syncBusyMark(): void {
  for (const button of document.querySelectorAll<HTMLElement>('button.tmt-ui-action'))
    if (!button.closest('#remotes, #remote-switcher')) markSlot(button);
  if (page.busy) {
    if (busyMark?.shownAt !== undefined && busyMark.timer !== undefined) {
      clearTimeout(busyMark.timer);
      busyMark.timer = undefined;
    }
    if (busyMark) return;
    const current: BusyMark = (busyMark = { owner: pressed ?? refresh });
    current.timer = setTimeout(() => {
      current.timer = undefined;
      current.shownAt = Date.now();
      markSlot(current.owner).dataset.busy = 'true';
    }, BUSY_MARK_DELAY_MS);
    return;
  }
  const current = busyMark;
  if (!current || (current.timer !== undefined && current.shownAt !== undefined)) return;
  if (current.shownAt === undefined) {
    clearTimeout(current.timer);
    busyMark = undefined;
    return;
  }
  current.timer = setTimeout(
    () => {
      markSlot(current.owner).dataset.busy = 'false';
      if (busyMark === current) busyMark = undefined;
    },
    Math.max(0, current.shownAt + BUSY_MARK_HOLD_MS - Date.now()),
  );
}
function saveState(form: string, hint: string, changed: boolean, valid: boolean): void {
  const button =
    element<HTMLFormElement>(form).querySelector<HTMLButtonElement>('button[type=submit]')!;
  fence(button, !page.writableWhenIdle || !changed || !valid);
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
    page.intent?.kind === 'setting'
      ? page.intent.input.setting
      : page.intent?.kind === 'remove-others'
        ? undefined
        : page.intent?.input.clientId;
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
    fence(recover, false);
    recover.setAttribute('aria-busy', String(page.busy));
  }
}

function render(): void {
  registryState(
    page.access === 'live'
      ? 'Connected'
      : page.access === 'lost'
        ? 'Access ended'
        : page.access === 'checking'
          ? 'Checking'
          : 'Unconfirmed',
  );
  if (confirmation) {
    const target = page.devices?.devices.find(
      (device) => device.clientId === confirmation!.clientId,
    );
    if (!page.writable || !target || targetSignature(target) !== confirmation.signature)
      cancelConfirmation();
  }
  renderFirestore(element('firestore-content'), page.firestoreAccess, page.firestore);
  const capacity = capacityCopy(page.accessRefusal);
  const access =
    capacity ??
    {
      checking: 'Checking current access…',
      live: 'Current browser access confirmed.',
      lost: 'Current browser access refused.',
      unconfirmed: 'Current browser access unconfirmed.',
    }[page.access];
  if (element('access').textContent !== access) element('access').textContent = access;
  const announcement =
    capacity ??
    {
      checking: '',
      live: 'Browser access confirmed',
      lost: 'Access ended',
      unconfirmed: 'Access unconfirmed',
    }[page.access];
  if (announcement && element('access-announcement').textContent !== announcement)
    element('access-announcement').textContent = announcement;
  const editable = page.access === 'live' && page.settings?.capabilities.settingsWrite === true;
  element('read-only').hidden = editable || page.access !== 'live';
  const designation = element('designation-command');
  designation.hidden = editable || page.access !== 'live';
  if (!designation.hidden)
    element('designation-code').textContent = `tmt remote devices designate ${page.clientId}`;
  element('access-notice').dataset.tone =
    page.access === 'live'
      ? editable
        ? 'working'
        : 'review'
      : page.access === 'checking'
        ? 'waiting'
        : 'blocked';
  const reason =
    capacity ??
    (!editable
      ? page.access === 'live'
        ? ''
        : 'Current browser access could not be confirmed.'
      : page.outcome?.state === 'unknown'
        ? 'Another change is still unconfirmed. Check its original result first.'
        : page.outcome?.state === 'refused' && page.outcome.reason === 'REMOTE_MANAGEMENT_CAPACITY'
          ? 'Browser change limit reached. Use the local CLI; do not retry or reset storage.'
          : '');
  element('controls-reason').textContent = reason;
  element('controls-reason').hidden = !reason;
  for (const button of document.querySelectorAll<HTMLButtonElement>('button')) {
    if (button.closest('#remotes, #remote-switcher')) continue;
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
  for (const button of document.querySelectorAll<HTMLButtonElement>(
    'form button:not(.settings-choice):not([role="option"])',
  ))
    fence(button, !page.writableWhenIdle);
  if (page.settings) {
    choices ??= [
      settingsChoice(opening, element('opening-options')),
      settingsChoice(mode, element('limit-mode-options')),
    ];
    for (const update of choices) update();
  }
  saveState('opening-form', 'opening-unchanged', openingChanged(), opening.checkValidity());
  saveState(
    'limit-form',
    'limit-unchanged',
    limitChanged(),
    mode.value !== 'custom' || custom.checkValidity(),
  );
  const removeOthers = element<HTMLButtonElement>('remove-others');
  removeOthers.hidden =
    page.removalPreview?.count === 0 ||
    (page.onFirstPage &&
      page.devices?.nextCursor === null &&
      page.devices.devices.every((device) => device.clientId === page.clientId));
  fence(removeOthers, !page.writableWhenIdle || page.removalPreview?.count === 0);
  const preview = page.removalPreview;
  const panel = element('remove-others-confirmation');
  panel.hidden = !preview || preview.count === 0 || !page.writable;
  removeOthers.setAttribute('aria-expanded', String(!panel.hidden));
  if (!panel.hidden && preview) {
    element('remove-others-consequence').textContent =
      `This ends Remote access for ${preview.count} other ${preview.count === 1 ? 'device' : 'devices'}. This browser stays connected. Press Remove the others again to confirm.`;
    removeOthers.setAttribute('aria-describedby', 'remove-others-consequence');
  }
  fence(refresh, false);
  more.hidden = !page.devices?.nextCursor;
  fence(more, false);
  first.hidden = page.onFirstPage;
  fence(first, false);
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
        revoke.dataset.action = 'revoke';
        revoke.className = 'tmt-ui-action';
        revoke.dataset.variant = 'destructive';
        const revokeLabel = document.createElement('span');
        revokeLabel.className = 'tmt-ui-action-label';
        revokeLabel.textContent = 'Remove';
        revoke.append(revokeLabel);
        revoke.addEventListener('click', (event) => {
          if (event.detail > 1 || !page.writable) return;
          // Confirmation is local presentation, never server authority.
          const target = page.devices?.devices.find((item) => item.clientId === device.clientId);
          if (!target || target.revoked) return;
          if (!confirmDevice(revoke, target, 'revoke')) return;
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
        talk.addEventListener('click', (event) => {
          if (event.detail > 1 || !page.writable) return;
          const target = page.devices?.devices.find((item) => item.clientId === device.clientId);
          if (!target || target.revoked) return;
          if (!target.talkEnabled && !confirmDevice(talk, target, 'talk')) return;
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
        const panel = document.createElement('div');
        panel.className = 'device-confirmation';
        panel.dataset.confirmation = '';
        panel.id = `confirm-${device.clientId}`;
        panel.hidden = true;
        const consequence = document.createElement('p');
        consequence.id = `${panel.id}-consequence`;
        consequence.setAttribute('role', 'status');
        const cancel = document.createElement('button');
        cancel.type = 'button';
        cancel.className = 'tmt-ui-action';
        const cancelLabel = document.createElement('span');
        cancelLabel.className = 'tmt-ui-action-label';
        cancelLabel.textContent = 'Cancel';
        cancel.append(cancelLabel);
        cancel.addEventListener('click', () => cancelConfirmation(true));
        panel.append(consequence, cancel);
        for (const button of [talk, revoke]) {
          button.setAttribute('aria-controls', panel.id);
          button.setAttribute('aria-expanded', 'false');
          button.addEventListener('keydown', (event) => {
            if ((event.repeat || event.isComposing) && ['Enter', ' '].includes(event.key))
              event.preventDefault();
          });
        }
        row.addEventListener('keydown', (event) => {
          if (event.key === 'Escape' && confirmation?.clientId === device.clientId) {
            event.preventDefault();
            cancelConfirmation(true);
          }
        });
        const disabledReason = document.createElement('p');
        disabledReason.id = `device-reason-${device.clientId}`;
        disabledReason.className = 'tmt-ui-field-description';
        row.append(summary, form, panel, deviceFeedback(device.clientId), disabledReason);
        rows.set(device.clientId, row);
        devices.append(row);
      }
      row.querySelector('.device-summary')!.textContent =
        `${device.name}${device.thisBrowser ? ' · This device' : ''} · ${device.kind} · ${device.revoked ? 'Revoked' : `Paired · Sending ${device.talkEnabled ? 'on' : 'off'}`} · ${device.liveSessionCount} live ${device.liveSessionCount === 1 ? 'session' : 'sessions'} · Last used ${device.lastUsedAtMs === null ? 'Not recorded' : new Date(device.lastUsedAtMs).toLocaleString()}`;
      row.querySelector('[data-action="talk"] .tmt-ui-action-label')!.textContent =
        device.talkEnabled ? 'Disable sending' : 'Enable sending';
      const form = row.querySelector('form')!;
      form.setAttribute('aria-label', `Device ${device.name}`);
      const name = row.querySelector<HTMLInputElement>('input')!;
      name.removeAttribute('aria-labelledby');
      name.setAttribute('aria-label', `Device name for ${device.name}`);
      // The target name comes from the admitted summary, never its unsent draft.
      row
        .querySelector('button[type=submit]')!
        .setAttribute(
          'aria-label',
          device.thisBrowser ? `Rename this device (${device.name})` : `Rename ${device.name}`,
        );
      row
        .querySelector('[data-action="talk"]')!
        .setAttribute(
          'aria-label',
          `${device.talkEnabled ? 'Disable' : 'Enable'} sending for ${device.name}`,
        );
      row
        .querySelector('button[data-action="revoke"]')!
        .setAttribute('aria-label', `Remove ${device.name}`);
      name.disabled = !editable || device.revoked;
      if (editable) name.removeAttribute('aria-describedby');
      else name.setAttribute('aria-describedby', 'read-only');
      const disabledReason = element(`device-reason-${device.clientId}`);
      disabledReason.textContent =
        page.outcome?.state === 'unknown' &&
        page.intent?.kind !== 'setting' &&
        page.intent?.kind !== 'remove-others' &&
        page.intent?.input.clientId === device.clientId
          ? ''
          : device.revoked
            ? 'This device is revoked.'
            : reason;
      disabledReason.hidden = !disabledReason.textContent;
      for (const button of row.querySelectorAll<HTMLButtonElement>('form button')) {
        fence(button, !page.writableWhenIdle || device.revoked);
        if (disabledReason.textContent) button.setAttribute('aria-describedby', disabledReason.id);
        else if (confirmation?.button === button)
          button.setAttribute('aria-describedby', `confirm-${device.clientId}-consequence`);
        else button.removeAttribute('aria-describedby');
        button.setAttribute('aria-busy', String(page.busy));
      }
    }
  }
  renderOutcome();
  syncBusyMark();
}
async function run(action: () => Promise<void>): Promise<void> {
  cancelConfirmation();
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
refresh.addEventListener('click', () => {
  if (page.busy) return;
  const ownedFocus = document.activeElement === refresh;
  let moved = false;
  const trackFocus = (event: Event) => {
    if (event.target !== refresh && event.target !== document.body) moved = true;
  };
  const trackPointer = (event: Event) => {
    if (!refresh.contains(event.target as Node)) moved = true;
  };
  document.addEventListener('focusin', trackFocus);
  document.addEventListener('pointerdown', trackPointer);
  void run(() => refreshView()).finally(() => {
    document.removeEventListener('focusin', trackFocus);
    document.removeEventListener('pointerdown', trackPointer);
    if (
      ownedFocus &&
      !moved &&
      refresh.isConnected &&
      !refresh.disabled &&
      (document.activeElement === document.body || document.activeElement === refresh)
    )
      refresh.focus({ preventScroll: true });
  });
});
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
element('copy-designation').addEventListener('click', () => {
  if (page.access !== 'live') return;
  void navigator.clipboard.writeText(`tmt remote devices designate ${page.clientId}`).then(
    () => {
      element('designation-copy-feedback').textContent = 'Copied.';
    },
    () => {
      element('designation-copy-feedback').textContent =
        'Copy failed. Select the command and copy it manually.';
    },
  );
});
element('remove-others').addEventListener('click', (event) => {
  if ((event as MouseEvent).detail > 1 || !page.writable) return;
  for (const device of page.devices?.devices ?? []) {
    const draft = rows.get(device.clientId)?.querySelector<HTMLInputElement>('input');
    if (draft && draft.value !== device.name) {
      page.notice = 'Save or restore the unsent device name before changing pages.';
      render();
      draft.focus();
      return;
    }
  }
  if (page.removalPreview && page.removalPreview.count > 0) {
    const fingerprint = page.removalPreview.fingerprint;
    page.removalPreview = undefined;
    void change({
      kind: 'remove-others',
      input: { operationId: crypto.randomUUID(), fingerprint },
    });
  } else {
    cancelConfirmation();
    void run(() => page.previewRemoval());
  }
});
element('cancel-remove-others').addEventListener('click', () => {
  page.removalPreview = undefined;
  render();
  element('remove-others').focus();
});
element('remove-others').addEventListener('keydown', (event) => {
  const key = event as KeyboardEvent;
  if ((key.repeat || key.isComposing) && ['Enter', ' '].includes(key.key)) event.preventDefault();
  if (key.key === 'Escape') {
    page.removalPreview = undefined;
    render();
  }
});
more.addEventListener('click', () => navigate(page.devices?.nextCursor ?? null));
first.addEventListener('click', () => navigate(null));
for (const button of document.querySelectorAll<HTMLButtonElement>('[data-recover]'))
  button.addEventListener('click', recoverOriginal);

try {
  let session = await reopenSession();
  page = new ManagementPage(management(session), async () => {
    session = await reopenSession(session, { retry: 'bounded' });
    return management(session);
  });
  await run(() => refreshView());
} catch (error) {
  const capacity = capacityCopy(error instanceof RefusalError ? error : undefined);
  const ended =
    error instanceof RefusalError &&
    [
      'REMOTE_DEVICE_REVOKED',
      'REMOTE_GRANT_EXPIRED',
      'REMOTE_SESSION_ENDED',
      'REMOTE_SESSION_EVICTED',
    ].includes(error.code);
  commandNotice(
    element('access'),
    capacity ??
      (ended
        ? 'Current browser access refused. Use the local CLI.'
        : "Remote may have stopped, or its reply couldn't be verified. Your pairing is unchanged."),
  );
  element('access-announcement').textContent =
    capacity ?? (ended ? 'Access ended' : 'Access unconfirmed');
  const verifiedEnded =
    error instanceof RefusalError &&
    ['REMOTE_DEVICE_REVOKED', 'REMOTE_GRANT_EXPIRED'].includes(error.code);
  registryState(verifiedEnded ? 'Access ended' : 'Unconfirmed');
  refresh.disabled = true;
  element('access-notice').dataset.tone = 'blocked';
  element('controls-reason').textContent =
    capacity ?? 'Current access is unavailable. Use the local CLI.';
}
