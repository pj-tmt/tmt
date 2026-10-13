/** Deterministic app data/Remote double. Never imported by production. */
import '@tmt/browser-ui/static.css';
import '../src/style.css';
import { createRoot, type Root } from 'react-dom/client';
import { StrictMode } from 'react';
import { RouterProvider } from '@tanstack/react-router';
import { ReadRefusedError } from '../src/ask-remote.js';
import type { AskBinding, PageAsk } from '../src/ask-panel.js';
import type { MessageAttachments, ThreadBinding } from '../src/thread-store.js';
import {
  AttachmentStaleError,
  AttachmentUploadError,
  type AttachmentBinding,
  type StoredAttachment,
} from '../src/attachment-service.js';
import type { FrozenAttachmentUpload } from '../src/attachment-channel.js';
import type { DocumentFiles } from '../src/document-files.js';
import { AttachmentReadError } from '../src/attachments.js';
import { ExportBundle } from '../src/export.js';
import type { QuoteSelector, ThreadView } from '../src/thread-records.js';
import type { PageView, PageBinding } from '../src/transport.js';
import { createAppRouter } from '../src/router.js';
import type { DraftStore } from '../src/draft-store.js';
import type { ComposerEdit } from '../src/components/message-composer-edit.js';
import type { AgentDestination } from '../src/live-ask.js';
import type { ManagementPort } from '../src/management.js';
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
let scope = 'epoch-1';
let pageEpoch = '1';
let openGate: (() => void) | undefined;
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
let presentationAgents: AgentDestination[] = [];
let presentationGate: Promise<void> | undefined;
let releasePresentation: (() => void) | undefined;
let rejectPresentation = false;
let presentationClient: AskBinding | undefined;
let presentationAsk: AskBinding | undefined;

export function finishMentionDirectory() {
  finishDirectory?.();
}
/** Reload-surviving stand-in for the encrypted device store (its crypto has unit tests). */
let releaseDraftRead: (() => void) | undefined;
export function finishDraftRead() {
  releaseDraftRead?.();
  releaseDraftRead = undefined;
}
function sessionDrafts(mode: 'session' | 'failing', wait?: Promise<void>): DraftStore {
  const name = (page: string) => `colab-fixture-drafts:${page}`;
  const read = (page: string) =>
    new Map<string, ComposerEdit>(JSON.parse(sessionStorage.getItem(name(page)) ?? '[]'));
  return {
    async load(page) {
      await wait;
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
  options: {
    creator?: boolean;
    /** Comment-capable binding without the independent Ask facade. */
    commentOnly?: boolean;
    /** A bounded preparation fault before any intent exists. */
    prepareFailures?: number;
    checking?: boolean;
    /** Explicit admitted-directory presentation states; production discovery remains unchanged. */
    agents?: AgentDestination[];
    drafts?: 'session' | 'failing';
    /** Hold the existing draft-store read to test shortcut lifetime fences. */
    holdDraftRead?: boolean;
    /** In-memory storage; a file name steers the outcome: `refuse`, `unknown`, `stale`. */
    attachments?: boolean;
    /** An Export panel with one included attachment and one of each outcome that includes none. */
    exportAttachments?: boolean;
    /** An admitted reply already present in the initial loader snapshot. */
    pastReplies?: boolean;
    /** Retain the committed answer until the test releases the verified projection. */
    holdQuestionView?: boolean;
    /** Presentation-only test port for the real Manage dialog. */
    management?: ManagementPort;
    strict?: boolean;
    /** Historical turns are part of the initial load, not a live arrival batch. */
    history?: 'thread' | 'chat';
  } = {},
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
  records = options.pastReplies
    ? [
        {
          operationId: id(21),
          requestId: `req_${id(901)}`,
          writer: id(4),
          agent: id(6),
          agentName: 'Agent 1',
          deviceName: 'You',
          issuedAt: Date.now() - 60_000,
          machine: id(5),
          message: 'An earlier question',
          state: 'accepted',
          canTrack: true,
          reply: 'A reply from before this page opened.',
        },
      ]
    : [];
  current = {
    source: '<p id="selected">Exact selected text</p>',
    title: 'Live shared page',
    own: {},
    asks: records,
    threads: [],
    publisherAgent: 'Agent 1',
    ...(options.creator
      ? { creationRecipient: { machineId: destination().machine, agentId: destination().agent } }
      : {}),
  };
  let preparationFailures = options.prepareFailures ?? 0;
  presentationAgents = [];
  presentationGate = undefined;
  releasePresentation = undefined;
  rejectPresentation = false;
  const ask: AskBinding = {
    async observeDestinations() {
      const agents = structuredClone(presentationAgents);
      const failed = rejectPresentation;
      if (presentationGate) await presentationGate;
      if (failed) throw new Error('Fixture directory unavailable');
      return { kind: 'ready', checkedAt: Date.now(), destinations: agents };
    },
    async destinations() {
      if (options.checking)
        await new Promise<void>((resolve) => {
          finishDirectory = resolve;
        });
      return (
        options.agents ??
        Array.from({ length: 5 }, (_, index) => ({
          ...destination(),
          agent: id(6 + index),
          agentName: `Agent ${index + 1}`,
          presence: index === 0 ? ('active' as const) : ('unknown' as const),
        }))
      );
    },
    async prepare(input) {
      if (preparationFailures > 0) {
        preparationFailures--;
        throw new Error('Fixture preparation unavailable');
      }
      // The fixture routes by hash, so location.href is not the mounted page URL a real app has.
      const fixture = fixtureAttempt(
        {
          ...selection(),
          ...input,
          url: pageLink(),
          ...(input.context
            ? { thread: input.context.thread.id, messageIds: [input.context.message.id] }
            : {}),
        },
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
  const stored = new Map<string, Uint8Array>();
  let staleOnce = true;
  let lostOnce = true;
  scope = 'epoch-1';
  pageEpoch = '1';
  const attachments: AttachmentBinding | undefined = options.attachments
    ? {
        scope: () => scope,
        async limits() {
          return { payloadBytes: 12 * 1024 * 1024 };
        },
        async upload(input, target, progress) {
          actions.push(`attach:upload:${input.filename}`);
          progress(1, 2);
          if (input.filename.includes('refuse'))
            throw new AttachmentUploadError({ kind: 'refused', reason: 'capacity' });
          const original = {
            transferId: crypto.randomUUID(),
            descriptor: {
              attachmentId: crypto.randomUUID(),
              epoch: pageEpoch,
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
          if (input.filename.includes('unknown') && lostOnce) {
            lostOnce = false;
            stored.set(original.descriptor.attachmentId, input.bytes.slice());
            throw new AttachmentUploadError({ kind: 'unknown', original });
          }
          progress(2, 2);
          stored.set(original.descriptor.attachmentId, input.bytes.slice());
          return { original, filename: input.filename, size: input.bytes.length };
        },
        async resume(original, input) {
          actions.push(`attach:resume:${original.transferId}`);
          return { original, filename: input.filename, size: input.size };
        },
        async discard(original) {
          actions.push(`attach:discard:${original.descriptor.filename}`);
        },
        async open(message, descriptor) {
          actions.push(
            `attach:open:${descriptor.filename}:${message.kind === 'message' ? message.revision : 'document'}`,
          );
          if (descriptor.filename.includes('slow')) await new Promise<void>((r) => (openGate = r));
          const bytes = stored.get(descriptor.attachmentId);
          for (const reason of ['denied', 'not-found', 'changed'] as const)
            if (descriptor.filename.startsWith(reason)) throw new AttachmentReadError(reason);
          if (!bytes || descriptor.filename.includes('missing')) throw new Error('Unavailable');
          return bytes.slice();
        },
      }
    : undefined;
  let staleFile = true;
  let lostSave = true;
  const files: DocumentFiles | undefined = attachments && {
    attachments,
    get epoch() {
      return pageEpoch;
    },
    target: () => ({ kind: 'document', source: current.source }),
    async add(list) {
      const names = list.map((s: StoredAttachment) => s.filename);
      if (names.some((name: string) => name.includes('stale')) && staleFile) {
        staleFile = false;
        throw new AttachmentStaleError(
          list.map((s: StoredAttachment) => s.original.descriptor.attachmentId),
        );
      }
      if (names.some((name: string) => name.includes('unsaved')) && lostSave) {
        lostSave = false;
        throw new Error('Save failed');
      }
      actions.push(`files:add:${names.join(',')}`);
      current = {
        ...current,
        attachments: [
          ...(current.attachments ?? []),
          ...list.map((s: StoredAttachment) => s.original.descriptor),
        ],
      };
      emit();
    },
    async remove(ids) {
      actions.push(`files:remove:${ids.length}`);
      current = {
        ...current,
        attachments: (current.attachments ?? []).filter((d) => !ids.includes(d.attachmentId)),
      };
      emit();
    },
  };
  function addTurn(body: string, thread: ThreadView, attach?: MessageAttachments) {
    if (attach?.stored.some((s: StoredAttachment) => s.filename.includes('stale')) && staleOnce) {
      staleOnce = false;
      throw new AttachmentStaleError(
        attach.stored.map((s: StoredAttachment) => s.original.descriptor.attachmentId),
      );
    }
    if (options.attachments) actions.push(`message:${body}:${attach?.stored.length ?? 0}`);
    const messageId = attach?.messageId ?? crypto.randomUUID();
    const {
      kind: _kind,
      threadId: _id,
      anchor: _anchor,
      resolved: _resolved,
      comments: _comments,
      ref: _ref,
      proposal: _proposal,
      decision: _decision,
      status: _status,
      notifications: _notifications,
      ...scope
    } = thread;
    const comment: ThreadView['comments'][number] = {
      ...scope,
      kind: 'comment',
      deviceAuthor: true,
      sequence: String(thread.comments.length + 1),
      thread: thread.ref,
      body,
      messageId,
      ...(attach
        ? { attachments: attach.stored.map((s: StoredAttachment) => s.original.descriptor) }
        : {}),
      ref: { writer: id(4), id: messageId },
    };
    current = {
      ...current,
      threads: [
        ...current.threads!.filter((value) => value.threadId !== thread.threadId),
        { ...thread, comments: [...thread.comments, comment] },
      ],
    };
    if (!options.holdQuestionView) emit();
    return {
      thread: thread.ref,
      threadRevision: thread.revision,
      message: { writer: id(4), id: messageId },
      messageRevision: '1',
    };
  }
  function createThread(
    body: string,
    anchor: QuoteSelector | null,
    threadId: string,
    attach?: MessageAttachments,
  ) {
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
    return addTurn(body, thread, attach);
  }
  const discussion: ThreadBinding = {
    attachments,
    async setStatus(ref, _previous, resolved) {
      actions.push(`status:${resolved ? 'resolve' : 'reopen'}`);
      return { changed: true, status: setResolved(find(ref), resolved, 'person') };
    },
    async decideProposal(ref, decision) {
      const thread = find(ref);
      if (!ownerDevice || !thread.proposal || thread.deleted || thread.decision)
        throw new Error('Decision unavailable');
      actions.push(`decision:${decision}`);
      const actionId = crypto.randomUUID();
      current = {
        ...current,
        threads: current.threads!.map((value) =>
          value === thread
            ? {
                ...value,
                decision: {
                  version: 1,
                  kind: 'proposal-decision',
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
                  previous: null,
                  decision,
                },
              }
            : value,
        ),
      };
      emit();
    },
    async notificationFailed() {
      throw new Error('Not used');
    },
    deviceId: id(4),
    async createChat(body, attach) {
      return createThread(body, null, id(4), attach);
    },
    async reply(ref, body, _expected, attach) {
      return addTurn(
        body,
        current.threads!.find((thread) => thread.ref.id === ref.id)!,
        attach,
      );
    },
    async create(body, anchor, attach) {
      return createThread(body, anchor, crypto.randomUUID(), attach);
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
        current.threads!.find((value) => value.anchor || value.proposal)!,
        true,
        'agent',
      );
    },
  };
  presentationAsk = ask;
  presentationClient = options.commentOnly ? undefined : ask;
  const binding: PageBinding = {
    get ask() {
      return presentationClient;
    },
    discussion,
    files,
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
    async edit() {
      return { merged: false };
    },
    async export() {
      if (!options.exportAttachments) throw new Error('Not used');
      const utf8 = (value: string) => new TextEncoder().encode(value);
      const file = `attachments/${id(40)}`;
      const info = (name: string, bytes: Uint8Array) => ({
        name,
        sizeBytes: bytes.length,
        sha256: '0'.repeat(64),
      });
      const [html, json, markdown, note, manifest] = [
        utf8('<p>page</p>'),
        utf8('{}'),
        utf8('# Conversations'),
        utf8('exported note'),
        utf8('{}'),
      ];
      return new ExportBundle(
        new Map([
          ['page.html', html],
          ['conversations.json', json],
          ['conversations.md', markdown],
          [file, note],
          ['manifest.json', manifest],
        ]),
        [
          info('page.html', html),
          info('conversations.json', json),
          info('conversations.md', markdown),
          info(file, note),
          info('manifest.json', manifest),
        ],
        [
          {
            attachmentId: id(40),
            filename: 'note.txt',
            plaintextBytes: 13,
            state: 'included',
            file,
          },
          { attachmentId: id(41), filename: 'gone.bin', plaintextBytes: 7, state: 'missing' },
          {
            attachmentId: id(42),
            filename: 'revoked.bin',
            plaintextBytes: 9,
            state: 'unavailable',
            reason: 'denied',
          },
          {
            attachmentId: id(43),
            filename: 'huge.bin',
            plaintextBytes: 13,
            state: 'unavailable',
            reason: 'too-large',
          },
        ],
      );
    },
    close() {},
  };
  if (options.history) {
    conversation({ surface: options.history, state: 'waiting' });
    scrollHistory();
  }
  root = createRoot(host);
  const draftRead = options.holdDraftRead
    ? new Promise<void>((resolve) => {
        releaseDraftRead = resolve;
      })
    : undefined;
  const app = (
    <RouterProvider
      router={createAppRouter({
        management: options.management,
        drafts: options.drafts ? sessionDrafts(options.drafts, draftRead) : undefined,
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
    />
  );
  root.render(options.strict ? <StrictMode>{app}</StrictMode> : app);
}
/** The local device loses owner-member provenance (a non-owner member). */
export function nonOwnerDevice() {
  ownerDevice = false;
  emit();
}
/** An anchored thread, as if a person had just saved it. */
/** The page's epoch or admission head moved (a revoke or an epoch advance). */
export function advanceDisclosure() {
  scope = `${scope}+`;
  emit();
}
/** An admitted document update removed every file while a read was pending. */
export function removePageFiles() {
  current = { ...current, attachments: [] };
  emit();
}
/** The page's epoch advanced: files sealed under the old one wait to be re-sealed. */
export function rotateEpoch() {
  pageEpoch = String(Number(pageEpoch) + 1);
  emit();
}
/** Lets a read of a file whose name contains `slow` finish. */
export function releaseOpen() {
  openGate?.();
}
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

/** Notification scenarios supply already admitted view data, never a Send result. */
export function notificationSources(
  value: Pick<PageView, 'asks' | 'threads' | 'askUnavailable'> & { source?: string },
) {
  records = structuredClone([...(value.asks ?? [])]);
  current = { ...current, ...structuredClone(value), asks: records };
  emit();
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
      ...(state === 'accepted' ? { requestId: `req_${id(901)}` } : {}),
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
      requestId: `req_${id(902)}`,
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
  reply?: string;
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
                : (options.reply ??
                  'Keep <script>reply</script> as text.\nThis is Atlas’s answer.'),
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

/** Authenticated proposal projection double; author HTML carries only its inert ID. */
export function proposal(placement: 'inline' | 'missing' | 'duplicate' = 'inline') {
  const proposalId = id(81);
  const placeholder = `<tmt-proposal data-id="${proposalId}"></tmt-proposal>`;
  current = {
    ...current,
    source: `<style>body {margin:24px;font:16px/1.5 sans-serif} tmt-proposal {display:block}</style><h1>Project notes</h1><p>Before the proposal.</p>${placement === 'missing' ? '' : placeholder}${placement === 'duplicate' ? placeholder : ''}<p id="after-proposal">The page continues here.</p>`,
    threads: [
      {
        version: 1,
        kind: 'thread',
        spaceId: selection().space,
        pageId: id(1),
        epoch: '1',
        senderDevice: id(4),
        deviceName: 'You',
        threadId: proposalId,
        revision: '1',
        at: String(Date.now()),
        anchor: null,
        resolved: false,
        deleted: false,
        ref: { writer: id(4), id: proposalId },
        comments: [],
        proposal: {
          proposalId,
          title: 'Use a clearer project heading',
          body: 'Change the heading to explain what the page contains. Keep the supporting notes below it.',
          proposer: {
            machineId: destination().machine,
            agentId: destination().agent,
            label: destination().agentName,
          },
        },
      },
    ],
  };
  emit();
}

/** Admitted own Ask projection for proposal delivery-state presentation cases. */
export function proposalAsk(state: PageAsk['state'] | 'replied') {
  const thread = current.threads!.find((thread) => thread.proposal)!;
  const comment = thread.comments.at(-1)!;
  current = {
    ...current,
    asks: [
      {
        thread: thread.threadId,
        messageIds: [comment.messageId],
        operationId: sends.sends[0].operationId,
        writer: id(4),
        message: comment.body,
        agent: destination().agent,
        agentName: destination().agentName,
        deviceName: 'You',
        machine: destination().machine,
        issuedAt: Date.now(),
        canTrack: true,
        state: state === 'replied' ? 'accepted' : state,
        ...(state === 'refused' ? { requestId: null, reason: 'REMOTE_SCOPE_DENIED' } : {}),
        ...(state === 'replied'
          ? {
              reply:
                'I will use the clearer heading.\n' +
                'The supporting notes stay in place. '.repeat(12),
            }
          : {}),
      },
    ],
  };
  emit();
}

/** Deterministic mixed admitted projection; no native publication proof. */
export function mixedProposals() {
  proposal('inline');
  const base = current.threads![0];
  const clone = (number: number, title: string): ThreadView => ({
    ...structuredClone(base),
    threadId: id(number),
    ref: { writer: base.ref.writer, id: id(number) },
    proposal: { ...base.proposal!, proposalId: id(number), title },
  });
  const approved = clone(82, 'Keep the supporting notes');
  approved.decision = {
    version: 1,
    kind: 'proposal-decision',
    spaceId: base.spaceId,
    pageId: base.pageId,
    epoch: base.epoch,
    senderDevice: base.senderDevice,
    deviceName: base.deviceName,
    revision: '1',
    at: base.at,
    deleted: false,
    actionId: id(101),
    thread: approved.ref,
    previous: null,
    decision: 'approved',
  };
  const resolved = clone(83, 'Confirm the page audience');
  resolved.resolved = true;
  const annotation: ThreadView = {
    ...structuredClone(base),
    threadId: id(91),
    ref: { writer: base.ref.writer, id: id(91) },
    proposal: undefined,
    deviceName: base.proposal!.proposer.label,
    anchor: { exact: 'Review the supporting notes.', prefix: '', suffix: '' },
  };
  const chat = {
    ...structuredClone(annotation),
    anchor: null,
    threadId: base.ref.writer,
    ref: { writer: base.ref.writer, id: base.ref.writer },
  };
  current = {
    ...current,
    title: 'Project review',
    source:
      '<style>body {margin:24px;font:16px/1.5 sans-serif}</style><h1>Project review</h1><p>Review the supporting notes.</p>' +
      [base, approved, resolved]
        .map((t) => `<tmt-proposal data-id="${t.threadId}"></tmt-proposal>`)
        .join('') +
      '<p id="after-proposal">The page continues here.</p>',
    threads: [base, annotation, approved, resolved, chat],
  };
  emit();
}

/** Trusted admitted-observation seam. Publishing a new operation uses the production trigger;
 * no fixture control reaches the authored page frame or changes a retained turn key. */
export function driverObservation(options: {
  driver?: 'claude' | 'codex';
  mismatch?: 'machine' | 'agent';
  held?: boolean;
  failed?: boolean;
}) {
  presentationAgents = [
    {
      ...destination(),
      presence: 'active',
      ...(options.driver ? { runningDriver: options.driver } : {}),
      ...(options.mismatch === 'machine' ? { machine: id(99) } : {}),
      ...(options.mismatch === 'agent' ? { agent: id(99) } : {}),
    },
  ];
  rejectPresentation = !!options.failed;
  presentationGate = options.held
    ? new Promise<void>((resolve) => {
        releasePresentation = resolve;
      })
    : undefined;
  // Separate operation with no reply triggers observation without replacing the first exchange.
  const seed = records[0]!;
  const trigger = {
    ...seed,
    operationId: crypto.randomUUID(),
    reply: undefined,
    messageIds: undefined,
    thread: undefined,
  };
  current = { ...current, asks: [...records, trigger] };
  emit();
}
export function settleDriverObservation() {
  releasePresentation?.();
  presentationGate = undefined;
  releasePresentation = undefined;
}

/** Same-client restoration is a scope change, not a new operation or a new turn. */
export function driverClient(options: {
  enabled: boolean;
  held?: boolean;
  driver?: 'claude' | 'codex';
}) {
  if (!presentationAsk) throw new Error('Fixture page not mounted');
  presentationAgents = [
    { ...destination(), ...(options.driver ? { runningDriver: options.driver } : {}) },
  ];
  rejectPresentation = false;
  presentationGate = options.held
    ? new Promise<void>((resolve) => {
        releasePresentation = resolve;
      })
    : undefined;
  presentationClient = options.enabled ? presentationAsk : undefined;
  emit();
}

/** Inert authored questions plus admitted thread projections; no publication proof. */
export function question(
  options: {
    kind?: 'single' | 'multiple' | 'short' | 'long';
    agent?: string;
    title?: string;
    choices?: string[];
    other?: boolean;
    placement?: 'inline' | 'duplicate' | 'nested';
    mixed?: boolean;
    darkPage?: boolean;
  } = {},
) {
  const escape = (value: string) =>
    value.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('"', '&quot;');
  const kind = options.kind ?? 'single';
  const choices =
    options.choices ?? (kind === 'single' || kind === 'multiple' ? ['Keep it', 'Change it'] : []);
  const markup = `<tmt-question data-id="${id(90)}" data-kind="${kind}" data-agent="${escape(options.agent ?? 'Agent 1')}" ${options.other ? 'data-other' : ''}><p data-question>${escape(options.title ?? 'Which heading should we use?')}</p>${choices.map((choice) => `<span data-choice>${escape(choice)}</span>`).join('')}</tmt-question>`;
  if (options.mixed) proposal('inline');
  current = {
    ...current,
    source: `<style>body{margin:24px;font:16px/1.5 sans-serif;background:${options.darkPage ? '#181818' : '#fff'};color:${options.darkPage ? '#eee' : '#222'}}tmt-question,tmt-proposal{display:block}</style><h1>Project notes</h1><p>Before the question.</p>${options.placement === 'nested' ? `<tmt-question data-id="${id(91)}">${markup}</tmt-question>` : markup}${options.placement === 'duplicate' ? markup : ''}<p id="after-question">The page continues here.</p>${options.mixed ? `<tmt-proposal data-id="${id(81)}"></tmt-proposal><p id="after-proposal">After both controls.</p>` : ''}`,
    threads: options.mixed ? current.threads : [],
  };
  emit();
}

/** Grouped authored markup uses the same parent parser and real quote/thread owners. */
export function questionGroup(
  options: { count?: number; darkPage?: boolean; longLabels?: boolean; lone?: boolean } = {},
) {
  const count = options.count ?? 2;
  const questions = Array.from({ length: count }, (_, index) => {
    const kind = index === 0 ? 'single' : index === 1 ? 'short' : 'long';
    const label = options.longLabels
      ? `Question ${String(index + 1).padStart(2, '0')}: page review`
      : `Q${index + 1}`;
    return `<tmt-question data-id="${id(100 + index)}" data-kind="${kind}" data-agent="Agent 1" data-label="${label}"><p data-question>${index === 0 ? 'Which heading should we use?' : index === 1 ? 'What should the introduction say?' : `What should section ${index + 1} explain?`}</p><p>Consider clarity and the intended reader.</p>${index === 0 ? '<span data-choice>Keep it</span><span data-choice>Change it</span>' : ''}</tmt-question>`;
  });
  current = {
    ...current,
    threads: [],
    asks: [],
    source: `<style>body{margin:24px;font:16px/1.5 sans-serif;background:${options.darkPage ? '#181818' : '#fff'};color:${options.darkPage ? '#eee' : '#222'}}</style><h1>Project notes</h1><tmt-questions data-id="${id(99)}"><p data-title>Review the page</p>${questions.join('')}</tmt-questions>${options.lone ? `<tmt-question data-id="${id(120)}" data-kind="long"><p data-question>Any other feedback?</p></tmt-question>` : ''}<p id="after-question">The page continues here.</p>`,
  };
  emit();
}

/** A reload restores only a verified projection, never local tab state or control text. */
export function saveQuestionPage() {
  const { source, title, threads, asks, archived } = current;
  sessionStorage.setItem(
    'colab-fixture-question-page',
    JSON.stringify({ source, title, threads, asks, archived }),
  );
}
export function restoreQuestionPage() {
  current = { ...current, ...JSON.parse(sessionStorage.getItem('colab-fixture-question-page')!) };
  emit();
}
export function releaseQuestionView() {
  emit();
}

/** Admitted answer Ask state attached to the same normal annotation turn. */
export function questionAsk(state: PageAsk['state'] | 'replied') {
  const thread = current.threads!.find((thread) => !thread.proposal)!;
  const comment = thread.comments.at(-1)!;
  current = {
    ...current,
    asks: [
      {
        thread: thread.threadId,
        messageIds: [comment.messageId],
        operationId: sends.sends[0].operationId,
        writer: id(4),
        message: comment.body,
        agent: destination().agent,
        agentName: destination().agentName,
        deviceName: 'You',
        machine: destination().machine,
        issuedAt: Date.now(),
        canTrack: true,
        state: state === 'replied' ? 'accepted' : state,
        ...(state === 'refused' ? { requestId: null, reason: 'REMOTE_SCOPE_DENIED' } : {}),
        ...(state === 'replied' ? { reply: 'I read your answer in this thread.' } : {}),
      },
    ],
  };
  emit();
}

/** Two verified reader streams can create sibling normal annotations concurrently. */
export function questionSibling(options: {
  body: string;
  sequence: string;
  device?: boolean;
  reverse?: boolean;
  index?: number;
  canonical?: boolean;
  empty?: boolean;
}) {
  const first = current.threads!.find((thread) => !thread.proposal && thread.comments.length)!;
  const threadId = id(92 + (options.index ?? 0) * 3),
    writer = options.canonical ? id(2) : id(93 + (options.index ?? 0) * 3),
    messageId = id(94 + (options.index ?? 0) * 3);
  const sibling: ThreadView = {
    ...first,
    threadId,
    ref: { writer, id: threadId },
    comments: options.empty
      ? []
      : [
          {
            ...first.comments[0],
            thread: { writer, id: threadId },
            messageId,
            ref: { writer, id: messageId },
            body: options.body,
            sequence: options.sequence,
            deviceAuthor: options.device !== false,
            deviceName: options.device === false ? 'Agent' : 'Other reader',
          },
        ],
  };
  const threads = [...current.threads!.filter((thread) => thread.ref.id !== threadId), sibling];
  current = { ...current, threads: options.reverse ? threads.reverse() : threads };
  emit();
}

/** A verified sync publication after an archive commit. */
export function archive() {
  current = { ...current, archived: true };
  emit();
}

/** Verified-projection presentation fixture for Chat identity and local-day headings. */
export function chatPolish(state: 'empty' | 'messages' | 'failed') {
  if (state === 'empty') {
    current = { ...current, threads: [], asks: [] };
    emit();
    return;
  }
  conversation({ surface: 'chat', state: state === 'failed' ? 'refused' : 'replied' });
  if (state === 'failed') {
    records[0].reason = 'REMOTE_SCOPE_DENIED';
    records[0].requestId = null;
  }
  const thread = current.threads![0];
  const own = thread.comments[0];
  const prior = new Date(Number(own.at));
  prior.setDate(prior.getDate() - 1);
  const messageId = id(45);
  const person = {
    ...own,
    at: String(prior.getTime()),
    messageId,
    senderDevice: id(46),
    deviceName: 'Pat',
    ref: { writer: id(46), id: messageId },
    body: 'Keep the conversation beside the selected page text.',
  };
  current = { ...current, threads: [{ ...thread, comments: [person, own] }] };
  emit();
}
