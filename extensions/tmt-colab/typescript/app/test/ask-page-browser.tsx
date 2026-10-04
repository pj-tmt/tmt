/** Deterministic app data/Remote double. Never imported by production. */
import { createRoot, type Root } from 'react-dom/client';
import { RouterProvider } from '@tanstack/react-router';
import { ReadRefusedError } from '../src/ask-remote.js';
import type { AskBinding, PageAsk } from '../src/ask-panel.js';
import type { ThreadBinding } from '../src/thread-store.js';
import type { ThreadView } from '../src/thread-records.js';
import type { PageView, PageBinding } from '../src/transport.js';
import { createAppRouter } from '../src/router.js';
import { fixtureAttempt } from './ask-browser-attempt.js';
import { destination, id, pageLink, RemoteDouble, selection } from './ask-fixtures.js';
let root: Root | undefined;
let sends: RemoteDouble;
let actions: string[];
let publish: ((value: PageView) => void) | undefined;
let fail: ((error: Error) => void) | undefined;
let records: PageAsk[];
let current: PageView;
let readRefusal: ConstructorParameters<typeof ReadRefusedError>[0] | undefined;
let prepareRelease: (() => void) | undefined;
let preparing: Promise<void> | undefined;
export async function mount() {
  root?.unmount();
  document.getElementById('ask-page-fixture')?.remove();
  document.getElementById('root')?.setAttribute('hidden', '');
  location.hash = '/pages/welcome';
  const host = document.createElement('div');
  host.id = 'ask-page-fixture';
  document.body.append(host);
  sends = new RemoteDouble();
  actions = [];
  readRefusal = undefined;
  records = [];
  current = {
    source: '<p id="selected">Exact selected text</p>',
    title: 'Live shared page',
    own: {},
    asks: [],
    threads: [],
    publisherAgent: 'Agent 1',
  };
  const ask: AskBinding = {
    async destinations() {
      return Array.from({ length: 5 }, (_, index) => ({
        ...destination(),
        agent: id(6 + index),
        agentName: `Agent ${index + 1}`,
        presence: index === 0 ? ('active' as const) : ('unknown' as const),
      }));
    },
    async prepare(input) {
      // The fixture routes by hash, so location.href is not the mounted page URL a real app has.
      const fixture = fixtureAttempt(
        { ...selection(), ...input, url: pageLink() },
        input.destination,
      );
      if (preparing) await preparing;
      const result = await fixture;
      sends = result.remote;
      return result.attempt;
    },
    async recheck(operationId) {
      actions.push(`recheck:${operationId}`);
      if (readRefusal) throw new ReadRefusedError(readRefusal);
    },
    async abandon(operationId) {
      actions.push(`abandon:${operationId}`);
    },
  };
  function addTurn(body: string, thread: ThreadView) {
    const messageId = crypto.randomUUID();
    const {
      kind: _kind,
      threadId: _id,
      anchor: _anchor,
      resolved: _resolved,
      comments: _comments,
      ref: _ref,
      ...scope
    } = thread;
    thread.comments.push({
      ...scope,
      kind: 'comment',
      thread: thread.ref,
      body,
      messageId,
      ref: { writer: id(4), id: messageId },
    });
    current = { ...current, threads: [thread] };
    publish?.(current);
    return {
      thread: thread.ref,
      threadRevision: thread.revision,
      message: { writer: id(4), id: messageId },
      messageRevision: '1',
    };
  }
  const discussion: ThreadBinding = {
    deviceId: id(4),
    async createChat(body) {
      const thread: ThreadView = {
        version: 1,
        kind: 'thread',
        spaceId: selection().space,
        pageId: id(1),
        epoch: '1',
        senderDevice: id(4),
        deviceName: 'You',
        threadId: id(4),
        revision: '1',
        at: String(Date.now()),
        anchor: null,
        resolved: false,
        deleted: false,
        ref: { writer: id(4), id: id(4) },
        comments: [],
      };
      return addTurn(body, thread);
    },
    async reply(ref, body) {
      return addTurn(
        body,
        current.threads!.find((thread) => thread.ref.id === ref.id)!,
      );
    },
    async create() {
      throw new Error('Not used');
    },
    async edit() {
      throw new Error('Not used');
    },
    async deleteComment() {
      throw new Error('Not used');
    },
    async updateThread() {
      throw new Error('Not used');
    },
  };
  const binding: PageBinding = {
    ask,
    discussion,
    subscribe(next, failed) {
      publish = next;
      fail = failed;
      next(current);
      return () => {
        publish = undefined;
        fail = undefined;
      };
    },
    async edit() {},
    async export() {
      throw new Error('Not used');
    },
    close() {},
  };
  root = createRoot(host);
  root.render(
    <RouterProvider
      router={createAppRouter({
        async spaceHome() {
          return { title: 'Fixture space', pages: [] };
        },
        async page() {
          return { id: id(1), sharing: 'private', ...current, binding };
        },
      })}
    />,
  );
}
export function proof() {
  return { sends: sends.sends, actions };
}
export function change(source: string) {
  current = { ...current, source, title: 'Changed live title' };
  publish?.(current);
}
export function block() {
  fail?.(new Error('Sync disconnected'));
}
export function pausePrepare() {
  preparing = new Promise<void>((resolve) => {
    prepareRelease = resolve;
  });
}
export function resumePrepare() {
  prepareRelease?.();
  preparing = undefined;
}
/** The admitted record of the ask this page just sent. */
export function syncSent() {
  syncRecords('accepted', sends.sends[0].operationId);
}
export function syncRecords(state: PageAsk['state'] = 'uncertain', operationId = id(21)) {
  records = [
    {
      operationId,
      writer: id(4),
      agent: id(6),
      agentName: 'Agent 1',
      deviceName: 'You',
      issuedAt: Date.now(),
      machine: id(5),
      message: '<script>inert ask</script>',
      state,
      canTrack: true,
      ...(state === 'accepted' ? { reply: '<img src=x onerror=alert(1)>' } : {}),
    },
    {
      operationId: id(22),
      writer: id(99),
      agent: id(7),
      agentName: 'Agent 2',
      deviceName: 'Alex’s browser',
      issuedAt: Date.now() - 5 * 60 * 1000,
      machine: id(5),
      message: 'Other member ask',
      state: 'accepted',
      canTrack: false,
      reply: '',
    },
  ];
  current = { ...current, asks: records };
  publish?.(current);
}

export function syncRefusal(reason: string) {
  syncRecords('refused');
  records[0].reason = reason;
  current = { ...current, asks: records };
  publish?.(current);
}

export function refuseRead(code: ConstructorParameters<typeof ReadRefusedError>[0]) {
  readRefusal = code;
}

export function pending() {
  syncRecords('accepted');
  delete records[0].reply;
  publish?.({ ...current, asks: [...records] });
}
