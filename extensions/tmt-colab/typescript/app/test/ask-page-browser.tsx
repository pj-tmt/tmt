/** Deterministic app data/Remote double. Never imported by production. */
import { createRoot, type Root } from 'react-dom/client';
import { RouterProvider } from '@tanstack/react-router';
import { ReadRefusedError } from '../src/ask-remote.js';
import type { AskBinding, PageAsk } from '../src/ask-panel.js';
import type { ThreadBinding } from '../src/thread-store.js';
import type { QuoteSelector, ThreadView } from '../src/thread-records.js';
import type { PageView, PageBinding } from '../src/transport.js';
import { createAppRouter } from '../src/router.js';
import type { DraftStore } from '../src/draft-store.js';
import type { ComposerEdit } from '../src/components/message-composer-edit.js';
import { fixtureAttempt } from './ask-browser-attempt.js';
import { ThreadStatusCoordinator } from '../src/thread-status-coordinator.js';
import { projectThreadPresentation } from '../src/thread-status-presentation.js';
import { ThreadStatusSeen } from '../src/thread-status-view.js';
import type { ThreadStatusView } from '../src/thread-status.js';
import { destination, id, pageLink, RemoteDouble, selection } from './ask-fixtures.js';
let root: Root | undefined;
let sends: RemoteDouble;
let dispatched: RemoteDouble[];
let actions: string[];
let publish: ((value: PageView) => void) | undefined;
let fail: ((error: Error) => void) | undefined;
let records: PageAsk[];
let current: PageView;
let seen: ThreadStatusSeen | undefined;
let seenOpens = 0;
let ownerDevice = true;
let hooks: { seed(): void; agentResolves(): void } | undefined;
/** Parent-computed presentation inputs, as Live publishes them. */
function emit() {
  current = {
    ...current,
    threadPresentations: (current.threads ?? []).map((thread) =>
      projectThreadPresentation(thread, current.asks ?? [], seen, ownerDevice),
    ),
  };
  publish?.(current);
}
let readRefusal: ConstructorParameters<typeof ReadRefusedError>[0] | undefined;
let prepareRelease: (() => void) | undefined;
let preparing: Promise<void> | undefined;
let finishDirectory: (() => void) | undefined;
export function finishMentionDirectory() {
  finishDirectory?.();
}
/** Reload-surviving stand-in for the encrypted device store (its crypto has unit tests). */
function sessionDrafts(mode: 'session' | 'failing'): DraftStore {
  const name = (page: string) => `colab-fixture-drafts:${page}`;
  const read = (page: string) =>
    new Map<string, ComposerEdit>(JSON.parse(sessionStorage.getItem(name(page)) ?? '[]'));
  return {
    async load(page) {
      return read(page);
    },
    async apply(page, changes) {
      if (mode === 'failing') return false;
      const drafts = read(page);
      for (const [key, edit] of changes) {
        drafts.delete(key);
        if (edit) drafts.set(key, edit);
      }
      sessionStorage.setItem(name(page), JSON.stringify([...drafts]));
      return true;
    },
  };
}
export async function mount(
  options: { creator?: boolean; checking?: boolean; drafts?: 'session' | 'failing' } = {},
) {
  root?.unmount();
  document.getElementById('ask-page-fixture')?.remove();
  document.getElementById('root')?.setAttribute('hidden', '');
  location.hash = '/pages/welcome';
  const host = document.createElement('div');
  host.id = 'ask-page-fixture';
  document.body.append(host);
  sends = new RemoteDouble();
  dispatched = [];
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
    ...(options.creator
      ? { creationRecipient: { machineId: destination().machine, agentId: destination().agent } }
      : {}),
  };
  const ask: AskBinding = {
    async destinations() {
      if (options.checking)
        await new Promise<void>((resolve) => {
          finishDirectory = resolve;
        });
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
      dispatched.push(sends);
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
    const comment: ThreadView['comments'][number] = {
      ...scope,
      kind: 'comment',
      thread: thread.ref,
      body,
      messageId,
      ref: { writer: id(4), id: messageId },
    };
    current = {
      ...current,
      threads: [
        ...current.threads!.filter((value) => value.threadId !== thread.threadId),
        { ...thread, comments: [...thread.comments, comment] },
      ],
    };
    emit();
    return {
      thread: thread.ref,
      threadRevision: thread.revision,
      message: { writer: id(4), id: messageId },
      messageRevision: '1',
    };
  }
  function createThread(body: string, anchor: QuoteSelector | null, threadId: string) {
    const thread: ThreadView = {
      version: 1,
      kind: 'thread',
      spaceId: selection().space,
      pageId: id(1),
      epoch: '1',
      senderDevice: id(4),
      deviceName: 'You',
      threadId,
      revision: '1',
      at: String(Date.now()),
      anchor,
      resolved: false,
      deleted: false,
      ref: { writer: id(4), id: threadId },
      comments: [],
    };
    return addTurn(body, thread);
  }
  const discussion: ThreadBinding = {
    async setStatus(ref, _previous, resolved) {
      actions.push(`status:${resolved ? 'resolve' : 'reopen'}`);
      return { changed: true, status: setResolved(find(ref), resolved, 'person') };
    },
    async notificationFailed() {
      throw new Error('Not used');
    },
    deviceId: id(4),
    async createChat(body) {
      return createThread(body, null, id(4));
    },
    async reply(ref, body) {
      return addTurn(
        body,
        current.threads!.find((thread) => thread.ref.id === ref.id)!,
      );
    },
    async create(body, anchor) {
      return createThread(body, anchor, crypto.randomUUID());
    },
    async edit() {
      throw new Error('Not used');
    },
    async deleteComment() {
      throw new Error('Not used');
    },
    async updateThread(ref, revision, change) {
      const thread = current.threads!.find((value) => value.ref.id === ref.id)!;
      if (thread.revision !== revision) throw new Error('Stale fixture revision');
      actions.push(`thread:${JSON.stringify(change)}`);
      current = {
        ...current,
        threads: [{ ...thread, ...change, revision: String(Number(revision) + 1) }],
      };
      emit();
    },
  };
  seenOpens = 0;
  ownerDevice = true;
  seen = new ThreadStatusSeen({ spaceId: selection().space, pageId: id(1), epoch: '1' }, id(4), {
    getItem: () => null,
    setItem: () => {},
  });
  const find = (ref: { writer: string; id: string }) =>
    current.threads!.find((value) => value.ref.writer === ref.writer && value.ref.id === ref.id)!;
  function statusView(
    thread: ThreadView,
    previous: ThreadStatusView | undefined,
    resolved: boolean,
    actor: 'person' | 'agent',
  ): ThreadStatusView {
    const actionId = crypto.randomUUID();
    return {
      version: 1,
      kind: 'thread-status',
      spaceId: thread.spaceId,
      pageId: thread.pageId,
      epoch: thread.epoch,
      senderDevice: id(4),
      deviceName: 'You',
      revision: '1',
      deleted: false,
      at: String(Date.now()),
      actionId,
      thread: thread.ref,
      previous: previous?.ref ?? null,
      resolved,
      actor,
      agentName: actor === 'agent' ? 'Atlas' : null,
      recipients: [],
      ref: { writer: id(4), id: actionId },
      depth: (previous?.depth ?? 0) + 1,
    };
  }
  function setResolved(thread: ThreadView, resolved: boolean, actor: 'person' | 'agent') {
    const status = statusView(thread, thread.status, resolved, actor);
    current = {
      ...current,
      threads: current.threads!.map((value) =>
        value === thread ? { ...value, resolved, status } : value,
      ),
    };
    emit();
    return status;
  }
  hooks = {
    seed() {
      void discussion.create('Opening note', {
        exact: 'Exact selected text',
        prefix: '',
        suffix: '',
      });
    },
    agentResolves() {
      setResolved(
        current.threads!.find((value) => value.anchor)!,
        true,
        'agent',
      );
    },
  };
  const binding: PageBinding = {
    ask,
    discussion,
    status: new ThreadStatusCoordinator({
      binding: discussion,
      asks: () => current.asks ?? [],
      destinations: async () => [],
      notify: async () => ({ adopted: true }),
    }),
    markThreadStatusSeen(ref) {
      seenOpens++;
      seen!.opened(find(ref));
      emit();
    },
    subscribe(next, failed) {
      publish = next;
      fail = failed;
      next(current);
      emit();
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
        drafts: options.drafts ? sessionDrafts(options.drafts) : undefined,
        async spaceHome() {
          return { title: 'Fixture space', pages: [] };
        },
        async page(pageId) {
          if (pageId === 'notes')
            return {
              id: id(2),
              sharing: 'private',
              title: 'Another fixture page',
              source: '<p id="other-page">Another fixture page</p>',
              own: {},
            };
          return {
            id: id(1),
            sharing: 'private',
            ...current,
            binding,
          };
        },
      })}
    />,
  );
}
/** The local device loses owner-member provenance (a non-owner member). */
export function nonOwnerDevice() {
  ownerDevice = false;
  emit();
}
/** An anchored thread, as if a person had just saved it. */
export function seedThread() {
  hooks!.seed();
}
/** The agent's winning resolution arrives from another stream. */
export function agentResolves() {
  hooks!.agentResolves();
}
/** How many times the parent's trusted open path acknowledged an agent resolution. */
export function seenProof() {
  return seenOpens;
}
export function proof() {
  return { sends: dispatched.flatMap((remote) => remote.sends), actions };
}
export function discussionProof() {
  return structuredClone(current.threads);
}
export function change(source: string) {
  current = { ...current, source, title: 'Changed live title' };
  emit();
}
export function block() {
  fail?.(new Error('Page unavailable'));
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
  emit();
}

export function syncRefusal(reason: string) {
  syncRecords('refused');
  records[0].reason = reason;
  current = { ...current, asks: records };
  emit();
}

export function refuseRead(code: ConstructorParameters<typeof ReadRefusedError>[0]) {
  readRefusal = code;
}

export function pending() {
  syncRecords('accepted');
  delete records[0].reply;
  publish?.({ ...current, asks: [...records] });
}

/** Admitted presentation double for shared Chat/annotation turns; no Remote effects. */
export function conversation(options: {
  surface: 'thread' | 'chat';
  state: PageAsk['state'] | 'waiting' | 'replied' | 'empty' | 'unavailable' | 'timeout';
  message?: string;
  agentName?: string;
}) {
  const at = String(
    Date.now() - (options.state === 'timeout' ? 2 * 60 * 60 * 1000 + 1 : 5 * 60 * 1000),
  );
  const thread: ThreadView = {
    version: 1,
    kind: 'thread',
    spaceId: selection().space,
    pageId: id(1),
    epoch: '1',
    senderDevice: id(4),
    deviceName: 'Asker browser',
    threadId: options.surface === 'chat' ? id(4) : id(41),
    revision: '1',
    at,
    anchor:
      options.surface === 'chat' ? null : { exact: 'Exact selected text', prefix: '', suffix: '' },
    resolved: false,
    deleted: false,
    ref: { writer: id(4), id: options.surface === 'chat' ? id(4) : id(41) },
    comments: [],
  };
  thread.comments.push({
    version: 1,
    kind: 'comment',
    spaceId: thread.spaceId,
    pageId: thread.pageId,
    epoch: '1',
    senderDevice: id(4),
    deviceName: thread.deviceName,
    messageId: id(42),
    revision: '1',
    at,
    thread: thread.ref,
    body:
      options.message ??
      'Explain <img src=x onerror=alert(1)> in this selection.\nKeep the exact text.',
    deleted: false,
    ref: { writer: id(4), id: id(42) },
  });
  const replied = options.state === 'replied' || options.state === 'empty';
  records = [
    {
      thread: thread.threadId,
      messageIds: [id(42)],
      operationId: id(43),
      writer: id(4),
      agent: id(6),
      agentName: options.agentName ?? 'Atlas',
      deviceName: 'Asker browser',
      issuedAt: Number(at),
      machine: id(5),
      message: 'Frozen ask bytes',
      state: ['replied', 'empty', 'waiting', 'unavailable', 'timeout'].includes(options.state)
        ? 'accepted'
        : (options.state as PageAsk['state']),
      ...(options.state === 'unavailable' ? { resultUnavailable: true } : {}),
      canTrack: true,
      ...(replied
        ? {
            reply:
              options.state === 'empty'
                ? ''
                : 'Keep <script>reply</script> as text.\nThis is Atlas’s answer.',
          }
        : {}),
    },
  ];
  current = { ...current, threads: [thread], asks: records };
  emit();
}

/** Admitted reply/history publication double; it never prepares or dispatches an Ask. */
export function windowReply(count = 0) {
  const thread = current.threads!.find((value) => value.anchor)!;
  const opening = thread.comments.at(-1)!;
  const comments = [...thread.comments];
  for (let index = 0; index < count; index++) {
    const messageId = crypto.randomUUID();
    comments.push({
      ...opening,
      ref: { ...opening.ref, id: messageId },
      messageId,
      body: `History turn ${index + 1}.`,
      at: String(Date.now()),
    });
  }
  const record: PageAsk = {
    thread: thread.threadId,
    messageIds: [opening.messageId],
    operationId: dispatched[0]?.sends[0]?.operationId ?? id(43),
    writer: opening.ref.writer,
    agent: id(6),
    agentName: 'Agent 1',
    deviceName: 'You',
    issuedAt: Date.now(),
    machine: id(5),
    message: opening.body,
    state: 'accepted',
    canTrack: true,
    reply: 'Exact associated agent reply.',
  };
  current = {
    ...current,
    threads: current.threads!.map((value) => (value === thread ? { ...thread, comments } : value)),
    asks: [record],
  };
  emit();
}

/** A source-independent publication still clones thread/Ask records like Live. */
export function unrelatedWindowUpdate() {
  current = { ...structuredClone(current), title: 'Unrelated live title' };
  emit();
}

/** Update the admitted reply on the same operation/thread, without another send. */
export function updateWindowReply() {
  current = {
    ...structuredClone(current),
    asks: current.asks!.map((record) => ({ ...record, reply: 'Updated associated agent reply.' })),
  };
  emit();
}

/** Long admitted history for both conversation surfaces, with stable identities. */
export function scrollHistory() {
  const thread = current.threads![0];
  const opening = thread.comments[0];
  current = {
    ...current,
    threads: [
      {
        ...thread,
        comments: Array.from({ length: 40 }, (_, index) => ({
          ...opening,
          messageId: id(100 + index),
          ref: { writer: id(7), id: id(100 + index) },
          senderDevice: id(7),
          deviceName: 'Other browser',
          body: `History turn ${index + 1}. Reading the shared page together.`,
        })),
      },
    ],
    asks: [],
  };
  emit();
}

/** Foreign comment or a newly admitted answer; never dispatches an Ask. */
export function scrollArrival(kind: 'comment' | 'reply' = 'comment') {
  const thread = current.threads![0];
  const last = thread.comments.at(-1)!;
  if (kind === 'comment') {
    const messageId = crypto.randomUUID();
    current = {
      ...current,
      threads: [
        {
          ...thread,
          comments: [
            ...thread.comments,
            {
              ...last,
              messageId,
              ref: { writer: id(7), id: messageId },
              body: 'A new turn from another browser.',
            },
          ],
        },
      ],
    };
  } else {
    current = {
      ...current,
      asks: [
        ...current.asks!,
        {
          thread: thread.threadId,
          messageIds: [last.messageId],
          operationId: crypto.randomUUID(),
          writer: last.ref.writer,
          agent: id(6),
          agentName: 'Atlas',
          deviceName: last.deviceName,
          issuedAt: Date.now(),
          machine: id(5),
          message: last.body,
          state: 'accepted',
          canTrack: true,
          reply: 'A newly admitted answer.',
        },
      ],
    };
  }
  emit();
}

/** Content growth is an edit, never a message identity arrival. */
export function resizeHistory() {
  const thread = current.threads![0];
  current = {
    ...current,
    threads: [
      {
        ...thread,
        comments: thread.comments.map((comment, index) =>
          index === thread.comments.length - 1
            ? {
                ...comment,
                body: `${comment.body}\n${'More detail in the same turn.\n'.repeat(60)}`,
              }
            : comment,
        ),
      },
    ],
  };
  emit();
}
