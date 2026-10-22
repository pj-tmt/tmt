/**
 * The design gallery's agent input frame (`/design/?frame=agent-input&state=…&theme=…`): the real Chat
 * panel and composer with the test doubles of `annotation-browser.tsx` under them. Nothing here sends: a busy or refused frame holds or refuses the write when the viewer presses Send
 * (the composer ignores a script's click).
 */
import 'virtual:tokens.css';
import '@tmt/browser-ui/static.css';
import '../src/colab-header.css';
import '../src/style.css';
import { createRoot } from 'react-dom/client';
import { BrowserDrawer } from '@tmt/browser-ui/react';
import {
  holdWrite,
  mount as mountFixture,
  setAgents,
  setAttachments,
  setCreator,
  setEchoHost,
  setHost,
  setInitialEdit,
  setSurface,
  setWriteFailure,
} from './annotation-browser.js';
import { AttachmentUploadError, type AttachmentBinding } from '../src/attachment-service.js';
import type { FrozenAttachmentUpload } from '../src/attachment-channel.js';
import { text } from '../src/strings.js';
import { destination, id } from './ask-fixtures.js';

/** What the frame shows: an empty input to type in, a long draft, a send in flight, or a refusal. */
export type AgentInputState = 'idle' | 'draft' | 'busy' | 'refused';
const STATES: readonly AgentInputState[] = ['idle', 'draft', 'busy', 'refused'];

const DRAFT = [
  'Please check the release notes before Friday.',
  '',
  'Two things I need from you:',
  '1. The migration order for the new device keys',
  '2. Whether the Remote door can stay paired during the upgrade',
].join('\n');

const QUESTION = 'Is the Friday release still on track?';

/** A draft that starts with the @astra chip, the recipient the page's creator default would seed. */
function addressed(rest: string) {
  const key = { machine: id(5), agent: id(6) };
  return {
    value: `@astra${rest || ' '}`,
    mentions: [{ key, range: { start: 0, end: 6 } }],
    edited: true,
  };
}

/** In-memory storage: any name uploads; a name with `refuse` is refused, as a full space would. */
function memoryAttachments(): AttachmentBinding {
  const stored = new Map<string, Uint8Array>();
  return {
    scope: () => 'epoch-1',
    async limits() {
      return { payloadBytes: 12 * 1024 * 1024 };
    },
    async upload(input, target, progress) {
      progress(1, 2);
      await new Promise((resolve) => setTimeout(resolve, 400));
      if (input.filename.includes('refuse'))
        throw new AttachmentUploadError({ kind: 'refused', reason: 'capacity' });
      const original = {
        transferId: crypto.randomUUID(),
        descriptor: {
          attachmentId: crypto.randomUUID(),
          epoch: '1',
          filename: input.filename,
          mediaType: input.mediaType,
          plaintextBytes: String(input.bytes.length),
          source:
            target.kind === 'message'
              ? {
                  kind: 'message',
                  writerId: id(4),
                  messageId: target.messageId,
                  messageRevision: '1',
                }
              : { kind: 'document', sourceDigest: '0'.repeat(64) },
        },
      } as unknown as FrozenAttachmentUpload;
      progress(2, 2);
      stored.set(original.descriptor.attachmentId, input.bytes.slice());
      return { original, filename: input.filename, size: input.bytes.length };
    },
    async resume(original, input) {
      return { original, filename: input.filename, size: input.size };
    },
    async discard() {},
    async open(_message, descriptor) {
      const bytes = stored.get(descriptor.attachmentId);
      if (!bytes) throw new Error('Unavailable');
      return bytes.slice();
    },
  };
}

/** The agents the @ popup offers: one online, one on another machine, one offline. */
function agents() {
  const base = destination();
  return [
    { ...base, agent: id(6), agentName: 'astra', presence: 'active' as const },
    {
      ...base,
      agent: id(7),
      agentName: 'atlas',
      machine: id(12),
      machineName: 'Office desktop',
      presence: 'active' as const,
    },
    { ...base, agent: id(8), agentName: 'reviewer', online: 'offline' as const },
  ];
}

/** Mounts one state of the agent input; the frame page supplies `params` (`state`). */
export function mount(root: HTMLElement, params: URLSearchParams = new URLSearchParams()) {
  const requested = params.get('state') as AgentInputState | null;
  const state = requested && STATES.includes(requested) ? requested : 'idle';
  setAgents(agents());
  setAttachments(memoryAttachments());
  setCreator(true);
  // The Chat shows a sent message at once, so a held write leaves the input as it is.
  setEchoHost(true);
  setSurface('chat');
  // A comment to everyone: the write it makes is what a busy or a refused frame holds or refuses.
  setInitialEdit(
    state === 'idle'
      ? addressed('')
      : state === 'draft'
        ? addressed(`\n${DRAFT}`)
        : { value: QUESTION, edited: true },
  );
  if (state === 'busy') holdWrite();
  if (state === 'refused') setWriteFailure('unavailable');
  // The Chat panel sits in the page's own Chat drawer, as it does on a Colab page.
  const body = document.createElement('div');
  // The fixture hides the page's own root, so the drawer gets a root of its own.
  root.hidden = true;
  const drawer = document.createElement('div');
  document.body.append(drawer);
  createRoot(drawer).render(
    <BrowserDrawer open title="Chat" panel="chat" caption={text.chatVisible} onDismiss={() => {}}>
      <div className="conversation-window" ref={(element) => element?.append(body)} />
    </BrowserDrawer>,
  );
  setHost(body);
  mountFixture(state === 'refused' ? 'write-failure' : 'held');
}
