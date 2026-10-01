import './popup.css';
import { captureActiveTab } from './capture.js';
import { journal, loadCapture } from './journal.js';
import { Intent } from './intent.js';
import { formatMessage, visibleText } from './message.js';
import type { Capture } from './message.js';
import { createStubClient } from './stub-client.js';
const client = createStubClient();
const intent = new Intent(client, journal);
const element = <T extends HTMLElement>(id: string): T => document.getElementById(id) as T;
const agent = element<HTMLSelectElement>('agent');
const note = element<HTMLTextAreaElement>('note');
const send = element<HTMLButtonElement>('send');
let capture: Capture | undefined;
let busy = false;
function message(): string {
  return intent.input?.message ?? (capture ? formatMessage(capture, note.value) : '');
}
function render(): void {
  let text = '';
  try {
    text = message();
  } catch (error) {
    showError(error);
  }
  element('preview').textContent = text || 'Capture a selection to begin.';
  element('escaped').textContent = visibleText(text);
  send.disabled = busy || !!intent.input || !text || !agent.value;
  agent.disabled = note.disabled = busy || !!intent.input;
  element<HTMLButtonElement>('capture').disabled = busy || !!intent.input;
  element<HTMLButtonElement>('recover').disabled = busy || !intent.input;
  element<HTMLButtonElement>('retry').hidden = intent.state?.state !== 'uncertain';
  element<HTMLButtonElement>('retry').disabled = busy;
  element<HTMLButtonElement>('reset').hidden = !intent.input;
  element<HTMLButtonElement>('reset').disabled = busy;
  if (intent.state) {
    const state = intent.state;
    element('status').textContent =
      state.state === 'held'
        ? 'Held · waiting for owner approval; no request yet.'
        : state.state === 'uncertain'
          ? 'Uncertain · check status before retrying.'
          : state.state === 'accepted'
            ? 'Accepted · checking the reply is safe.'
            : `${state.state} · ${state.reason ?? ''}`;
  }
  element('reply').textContent =
    intent.result?.state === 'replied'
      ? intent.result.message === ''
        ? '(Empty reply)'
        : intent.result.message
      : '';
  if (intent.errorCode === 'unpaired')
    element('status').textContent = 'Client unpaired · pair before continuing; no automatic retry.';
  if (intent.result)
    element('status').textContent =
      intent.result.state === 'replied'
        ? 'Replied'
        : intent.result.state === 'pending'
          ? 'Reply pending'
          : 'Result unavailable · do not resend.';
}
function showError(error: unknown): void {
  const codes = [
    'unpaired',
    'closed',
    'scope_denied',
    'rate_limited',
    'input_invalid',
    'unavailable',
  ];
  const code = error && typeof error === 'object' && 'code' in error ? String(error.code) : '';
  element('status').textContent = codes.includes(code)
    ? `Client ${code.replaceAll('_', ' ')}. Check the client before continuing.`
    : 'This action is unavailable. Check the selection, page permissions and local storage, then check status before retrying.';
}
async function action(event: Event, run: () => Promise<void>): Promise<void> {
  event.preventDefault();
  if (!event.isTrusted || busy) return;
  busy = true;
  render();
  let failure: unknown;
  try {
    await run();
  } catch (error) {
    failure = error;
  } finally {
    busy = false;
    render();
    if (failure) showError(failure);
  }
}
element('capture').addEventListener(
  'click',
  (event) =>
    void action(event, async () => {
      capture = undefined;
      capture = await captureActiveTab();
      element('status').textContent = 'Review the exact message, then Send.';
    }),
);
element('composer').addEventListener('submit', (event) => event.preventDefault());
send.addEventListener(
  'click',
  (event) =>
    void action(event, async () => {
      if (!capture || intent.input) return;
      await intent.send({
        operationId: crypto.randomUUID(),
        agentId: agent.value,
        message: message(),
      });
    }),
);
element('recover').addEventListener('click', (event) => void action(event, () => intent.recover()));
element('retry').addEventListener(
  'click',
  (event) =>
    void action(event, async () => {
      if (
        intent.state?.state === 'uncertain' &&
        confirm('Retry this exact message with the same operation ID?')
      )
        await intent.send();
    }),
);
element('reset').addEventListener(
  'click',
  (event) =>
    void action(event, async () => {
      if (
        !confirm(
          'Start a new message? This forgets the local preview; it does not cancel submitted work.',
        )
      )
        return;
      await intent.clear();
      capture = undefined;
      note.value = '';
      element('status').textContent = 'Capture a new selection.';
    }),
);
note.addEventListener('input', render);
agent.addEventListener('change', render);
async function start(): Promise<void> {
  await intent.restore();
  for (const item of await client.listAgents()) agent.add(new Option(item.name, item.id));
  if (intent.input) agent.value = intent.input.agentId;
  capture = await loadCapture();
  render();
}
void start().catch(showError);
