import { pendingVectorData, type PendingVector } from './pending-vector.js';
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
import type { Projection } from '../src/fold-protocol.js';
import type { PageView, PageBinding, PageTransport } from '../src/transport.js';
import { createAppRouter } from '../src/router.js';
import type { DraftStore } from '../src/draft-store.js';
import type { ComposerEdit } from '../src/components/message-composer-edit.js';
import type { AgentDestination } from '../src/live-ask.js';
import type { ManagementPort } from '../src/management.js';
import { commentsForAnswerBatch } from '../src/answer-batch.js';
import { inspectQuestions } from '../src/questions.js';
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
let answerAttempts: {
  input: Parameters<AskBinding['prepare']>[0];
  selection: ReturnType<typeof selection>;
  operationId: string;
}[];
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
let failNextStatus = false;
let releaseStatusNotice: (() => void) | undefined;
export function finishStatusNotice() {
  releaseStatusNotice?.();
  releaseStatusNotice = undefined;
}
/** The next status action fails before it is published, like a lost connection. */
let sourceSaves = 0;
/** The next Source save waits for `releaseEdit`, as a slow Save does. */
let editGate: { held?: Promise<void>; release?(): void } = {};
export function holdEdit() {
  editGate.held = new Promise<void>((resolve) => (editGate.release = resolve));
}
export function releaseEdit() {
  editGate.release?.();
  editGate = {};
}
let pageLoads = 0;
/** Page snapshots the route or a reopen asked the transport for. */
export function pageLoadCount() {
  return pageLoads;
}
/** Source saves the page asked the binding for. */
export function sourceSaveCount() {
  return sourceSaves;
}
/** The binding reports a stale epoch, so the page reopens the same page with a new binding. */
export function staleEpoch() {
  fail?.(new Error('STALE_EPOCH'));
}
export function failStatusOnce() {
  failNextStatus = true;
}
let holdNextStatus = false;
let heldStatus: { release(fail: boolean): void; settled: Promise<void> } | undefined;
/** The next status change waits for `releaseStatus`. */
export function holdStatusOnce() {
  holdNextStatus = true;
}
/** Settles the held status change, then waits until the page has rendered its outcome. */
export async function releaseStatus(options: { fail?: boolean } = {}) {
  const held = heldStatus!;
  heldStatus = undefined;
  held.release(!!options.fail);
  await held.settled;
  await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
}
let releaseQuestionDirectory: (() => void) | undefined;
export function finishQuestionDirectory() {
  releaseQuestionDirectory?.();
  releaseQuestionDirectory = undefined;
}
let presentationAgents: AgentDestination[] = [];
let presentationGate: Promise<void> | undefined;
let releasePresentation: (() => void) | undefined;
let rejectPresentation = false;
let presentationClient: AskBinding | undefined;
let presentationAsk: AskBinding | undefined;

export function finishMentionDirectory() {
  finishDirectory?.();
}
let holdAttachmentUploads = false;
let releaseAttachmentUpload: (() => void) | undefined;
let replacedFiles = false;
/** Reload-surviving stand-in for the encrypted device store (its crypto has unit tests). */
let releaseDraftRead: (() => void) | undefined;
let releaseDraftWrites: (() => void) | undefined;
export function finishDraftRead() {
  releaseDraftRead?.();
  releaseDraftRead = undefined;
}
/** Lets the held draft writes of the first page finish (the `split` store). */
export function releaseDraftWrite() {
  releaseDraftWrites?.();
}
/** `split`: the first page's writes wait for `releaseDraftWrite` and then succeed; every other
 * page's writes fail. */
function sessionDrafts(mode: 'session' | 'failing' | 'split', wait?: Promise<void>): DraftStore {
  const held =
    mode === 'split'
      ? new Promise<void>((resolve) => {
          releaseDraftWrites = resolve;
        })
      : undefined;
  const name = (page: string) => `colab-fixture-drafts:${page}`;
  const read = (page: string) =>
    new Map<string, ComposerEdit>(JSON.parse(sessionStorage.getItem(name(page)) ?? '[]'));
  return {
    async load(page) {
      await wait;
      return read(page);
    },
    async apply(page, changes) {
      if (mode === 'failing' || (mode === 'split' && page !== id(1))) return false;
      await held;
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
    /** Real fixture ledger refusals, so reload retries exercise the same admission as production. */
    refuseAsks?: number;
    checking?: boolean;
    /** Explicit admitted-directory presentation states; production discovery remains unchanged. */
    agents?: AgentDestination[];
    /** Names the observed directory lists, as it does for a viewer who holds no Ask record. */
    presentation?: readonly string[];
    agentsUnavailable?: boolean;
    /** One barrier shared by all destination reads in this mounted question page. */
    holdQuestionDirectory?: boolean;
    /** The status coordinator resolves mentions against the directory and records each notice. */
    statusAgents?: boolean;
    /** Hold only notification completion, after the ordinary record and status are admitted. */
    holdStatusNotice?: boolean;
    drafts?: 'session' | 'failing' | 'split';
    /** Hold the existing draft-store read to test shortcut lifetime fences. */
    holdDraftRead?: boolean;
    /** In-memory storage; a file name steers the outcome: `refuse`, `unknown`, `stale`. */
    attachments?: boolean;
    /** An Export panel with one included attachment and one of each outcome that includes none. */
    exportAttachments?: boolean;
    /** An admitted reply already present in the initial loader snapshot. */
    pastReplies?: boolean;
    /** Restore saved question history before the first loader/binding view. */
    restoreQuestionPage?: boolean;
    /** Retain the committed answer until the test releases the verified projection. */
    holdQuestionView?: boolean;
    /** One answer publication fails, before commit or after a deliberately lost acknowledgement. */
    failAnswerAt?: number;
    recordBeforeFail?: boolean;
    /** Presentation-only test port for the real Manage dialog. */
    management?: ManagementPort;
    strict?: boolean;
    /** Historical turns are part of the initial load, not a live arrival batch. */
    history?: 'thread' | 'chat';
    /** The page's layout and, for a canvas, its reason; absent is `freeform`. */
    layout?: Projection['layout'];
    layoutReason?: string;
  } = {},
) {
  resumeAttachmentUploads();
  finishStatusNotice();
  finishQuestionDirectory();
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
  answerAttempts = [];
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
    ...(options.layout ? { layout: options.layout } : {}),
    ...(options.layoutReason ? { layoutReason: options.layoutReason } : {}),
    ...(options.creator
      ? { creationRecipient: { machineId: destination().machine, agentId: destination().agent } }
      : {}),
  };
  let preparationFailures = options.prepareFailures ?? 0;
  let askRefusals = options.refuseAsks ?? 0;
  presentationAgents = (options.presentation ?? []).map((agentName) => ({
    ...destination(),
    agentName,
    presence: 'active' as const,
  }));
  presentationGate = undefined;
  releasePresentation = undefined;
  rejectPresentation = false;
  const questionDirectory = options.holdQuestionDirectory
    ? new Promise<void>((resolve) => {
        releaseQuestionDirectory = resolve;
      })
    : undefined;
  const ask: AskBinding = {
    async observeDestinations() {
      const agents = structuredClone(presentationAgents);
      const failed = rejectPresentation;
      if (presentationGate) await presentationGate;
      if (failed) throw new Error('Fixture directory unavailable');
      return { kind: 'ready', checkedAt: Date.now(), destinations: agents };
    },
    async destinations() {
      await questionDirectory;
      if (options.agentsUnavailable) throw new Error('Fixture directory unavailable');
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
      const captured = {
        ...selection(),
        ...input,
        url: pageLink(),
        ...(input.batch
          ? commentsForAnswerBatch(
              [...(current.threads ?? [])],
              input.batch,
              inspectQuestions(current.source),
              id(4),
            )
          : input.context
            ? { thread: input.context.thread.id, messageIds: [input.context.message.id] }
            : {}),
      };
      const fixture = fixtureAttempt(captured, input.destination, {
        retryOf: input.retryOf,
        mode: askRefusals-- > 0 ? 'refused' : 'accepted',
      });
      if (preparing) await preparing;
      const result = await fixture;
      sends = result.remote;
      dispatched.push(sends);
      if (input.context && input.comment.startsWith('Answer to “'))
        answerAttempts.push({
          input: structuredClone(input),
          selection: structuredClone(captured),
          operationId: result.attempt.preview.view.operationId,
        });
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
  replacedFiles = false;
  const attachments: AttachmentBinding | undefined = options.attachments
    ? {
        scope: () => scope,
        async limits() {
          return { payloadBytes: 12 * 1024 * 1024 };
        },
        async upload(input, target, progress) {
          actions.push(`attach:upload:${input.filename}`);
          progress(1, 2);
          if (holdAttachmentUploads)
            await new Promise<void>((resolve) => {
              releaseAttachmentUpload = resolve;
            });
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
  // A replacement owner of the same page's files: it records its own uploads and targets, then
  // delegates, so a test tells which owner an upload went through.
  const replacementFiles: DocumentFiles | undefined = files && {
    attachments: {
      ...files.attachments,
      async upload(input, target, progress) {
        actions.push(`files-b:upload:${input.filename}`);
        return files.attachments.upload(input, target, progress);
      },
    },
    get epoch() {
      return files.epoch;
    },
    target() {
      actions.push('files-b:target');
      return files.target();
    },
    add: (list) => files.add(list),
    remove: (ids) => files.remove(ids),
  };
  let answerPublications = 0;
  function publishAnswer<T>(body: string, commit: () => T): T {
    if (body.startsWith('Answer to “') && ++answerPublications === options.failAnswerAt) {
      if (options.recordBeforeFail) commit();
      throw new Error('Fixture answer publication was not confirmed');
    }
    return commit();
  }
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
      createdAt: _createdAt,
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
      createdAt: String(Date.now()),
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
    async setStatus(ref, _previous, resolved, recipients) {
      if (holdNextStatus) {
        holdNextStatus = false;
        let settle!: () => void;
        const settled = new Promise<void>((resolve) => (settle = resolve));
        const fail = await new Promise<boolean>((release) => {
          heldStatus = { release, settled };
        });
        // The caller's own continuation runs before the page renders its outcome.
        queueMicrotask(settle);
        if (fail) throw new Error('Fixture status publication was not confirmed');
      }
      if (failNextStatus) {
        failNextStatus = false;
        throw new Error('Fixture status publication was not confirmed');
      }
      actions.push(`status:${resolved ? 'resolve' : 'reopen'}`);
      return {
        changed: true,
        status: setResolved(
          find(ref),
          resolved,
          'person',
          structuredClone([...(recipients ?? [])]),
        ),
      };
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
      await replyGate.held;
      if (replyGate.fail) {
        replyGate.fail = false;
        throw new Error('Chat message refused');
      }
      return createThread(body, null, id(4), attach);
    },
    async reply(ref, body, _expected, attach) {
      await replyGate.held;
      if (replyGate.fail) {
        replyGate.fail = false;
        throw new Error('Reply refused');
      }
      return publishAnswer(body, () =>
        addTurn(
          body,
          current.threads!.find((thread) => thread.ref.id === ref.id)!,
          attach,
        ),
      );
    },
    async create(body, anchor, attach) {
      return publishAnswer(body, () => createThread(body, anchor, crypto.randomUUID(), attach));
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
    recipients: ThreadStatusView['recipients'] = [],
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
      recipients,
      ref: { writer: id(4), id: actionId },
      depth: (previous?.depth ?? 0) + 1,
    };
  }
  function setResolved(
    thread: ThreadView,
    resolved: boolean,
    actor: 'person' | 'agent',
    recipients: ThreadStatusView['recipients'] = [],
  ) {
    const status = statusView(thread, thread.status, resolved, actor, recipients);
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
  const statusNotice = options.holdStatusNotice
    ? new Promise<void>((resolve) => {
        releaseStatusNotice = resolve;
      })
    : undefined;
  const binding: PageBinding = {
    get ask() {
      return presentationClient;
    },
    discussion,
    files,
    status: new ThreadStatusCoordinator({
      binding: discussion,
      asks: () => current.asks ?? [],
      destinations: async () => (options.statusAgents ? await ask.destinations() : []),
      notify: async (_status, recipient) => {
        if (options.statusAgents) actions.push(`notify:${recipient.agentName}`);
        await statusNotice;
        return { adopted: true };
      },
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
        if (publish === next) publish = undefined;
        if (fail === failed) fail = undefined;
      };
    },
    async edit() {
      sourceSaves++;
      await editGate.held;
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
  if (options.restoreQuestionPage) restoreQuestionPage();
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
  const secondPage = '<p id="review-b-page">Second page</p>';
  const replacedBinding = Object.defineProperties({} as PageBinding, {
    ...Object.getOwnPropertyDescriptors(binding),
    files: { value: replacementFiles },
  });
  const transport: PageTransport = {
    management: options.management,
    drafts: options.drafts ? sessionDrafts(options.drafts, draftRead) : undefined,
    async spaceHome() {
      return {
        title: 'Fixture space',
        pages: [],
      };
    },
    async page(pageId) {
      pageLoads++;
      // The same review source on a different page, with its own discussion binding.
      if (pageId === 'review-b' || pageId === id(3))
        return {
          id: id(3),
          sharing: 'private',
          ...current,
          source: `${current.source}${secondPage}`,
          binding: {
            ...binding,
            discussion: { ...discussion },
            subscribe: (next, failed) =>
              binding.subscribe(
                (view) => next({ ...view, source: `${view.source}${secondPage}` }),
                failed,
              ),
          },
        };
      if (pageId === 'notes')
        return {
          id: id(2),
          sharing: 'private',
          title: 'Another fixture page',
          source: '<p id="other-page">Another fixture page</p>',
          own: {},
          // The split draft store needs a composer on the second page too.
          ...(options.drafts === 'split' ? { binding } : {}),
        };
      return {
        id: id(1),
        sharing: 'private',
        ...current,
        binding: replacedFiles ? replacedBinding : binding,
      };
    },
  };
  const app = <RouterProvider router={createAppRouter(transport)} />;
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
export function answerBatchProof() {
  return structuredClone(answerAttempts);
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
/** A page of exactly this markup, in the layout `mount` chose. */
export function authored(source: string) {
  current = { ...current, threads: [], asks: [], source };
  emit();
}
/** A live, metadata-only change: the source stays and the page's layout (and reason) change. */
export function setLayout(next: { layout?: Projection['layout']; layoutReason?: string }) {
  const { layout: _layout, layoutReason: _reason, ...rest } = current;
  current = {
    ...rest,
    ...(next.layout ? { layout: next.layout } : {}),
    ...(next.layoutReason ? { layoutReason: next.layoutReason } : {}),
  };
  emit();
}
export function change(source: string) {
  current = { ...current, source, title: 'Changed live title' };
  emit();
}
/** The same page reopens with a binding whose files have a replacement owner. */
export function replaceFilesOwner() {
  replacedFiles = true;
  fail?.(new Error('STALE_EPOCH'));
}
/** Holds or refuses the next reply or Chat write, as a slow or refused Remote append would. */
let replyGate: { held?: Promise<void>; release?(): void; fail?: boolean } = {};
export function holdReply() {
  replyGate.held = new Promise<void>((resolve) => (replyGate.release = resolve));
}
export function releaseReply() {
  replyGate.release?.();
  replyGate = { fail: replyGate.fail };
}
export function failNextReply() {
  replyGate.fail = true;
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
    createdAt: at,
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
  current = { ...current, threads: [thread], asks: records, agentActivity: undefined };
  emit();
}

/** Page-owned cosmetic observation, published through the real router projection. */
export function conversationActivity(options: {
  kind: 'sent' | 'working' | 'queued' | 'finished' | 'unknown';
  active?: boolean;
  ageMs?: number;
  chipState?: 'working' | 'idle' | 'unknown' | 'ended';
}) {
  const record = current.asks![0];
  current = {
    ...current,
    agentActivity: {
      active: options.active ?? true,
      receivedAt: performance.now() - (options.ageMs ?? 0),
      intervalMs: 60_000,
      agents: options.chipState
        ? [
            {
              machine: record.machine,
              machineName: 'This machine',
              agent: record.agent,
              name: record.agentName,
              state: options.chipState,
            },
          ]
        : [],
      entries: [
        {
          operationId: record.operationId,
          machine: record.machine,
          agent: record.agent,
          kind: options.kind,
          elapsedMs: 185000,
        },
      ],
    },
  };
  emit();
}

/** One anchored thread on paragraph `index` of `largePage`, with a single opening comment. */
function largeThread(index: number, comments = 1): ThreadView {
  // Fixed times, so a thread keeps its place in the page's numbering when a reply arrives.
  const at = String(1_760_000_000_000 + index * 1000);
  const threadId = id(1000 + index);
  const exact = `Paragraph ${index} has its own unique sentence`;
  const thread: ThreadView = {
    version: 1,
    kind: 'thread',
    spaceId: selection().space,
    pageId: id(1),
    epoch: '1',
    senderDevice: id(4),
    deviceName: 'Asker browser',
    threadId,
    revision: '1',
    at,
    createdAt: at,
    anchor: { exact, prefix: '', suffix: '' },
    resolved: false,
    deleted: false,
    ref: { writer: id(4), id: threadId },
    comments: [],
  };
  for (let n = 0; n < comments; n++) {
    const messageId = id(100_000 + index * 100 + n);
    thread.comments.push({
      version: 1,
      kind: 'comment',
      spaceId: thread.spaceId,
      pageId: thread.pageId,
      epoch: '1',
      senderDevice: id(4),
      deviceName: thread.deviceName,
      messageId,
      revision: '1',
      at,
      thread: thread.ref,
      body: n === 0 ? `Comment on paragraph ${index}.` : `Reply ${n} on paragraph ${index}.`,
      deleted: false,
      ref: { writer: id(4), id: messageId },
    });
  }
  return thread;
}
/** A large page: `paragraphs` paragraphs of prose with a thread on each of the first `threads`. */
export function largePage(options: { paragraphs: number; threads: number }) {
  const source = Array.from(
    { length: options.paragraphs },
    (_, index) =>
      `<p id="para-${index}">Paragraph ${index} has its own unique sentence, followed by enough ordinary words to make every block a realistic line of prose for a page that has grown over many sessions.</p>`,
  ).join('\n');
  current = {
    ...current,
    source,
    threads: Array.from({ length: options.threads }, (_, index) => largeThread(index)),
    asks: [],
  };
  emit();
}
/** An admitted reply on thread `index`, published as Live does: a fresh clone of the projection. */
export function largePageReply(index: number) {
  current = structuredClone({
    ...current,
    threads: current.threads!.map((thread, at) => (at === index ? largeThread(index, 2) : thread)),
  });
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
        createdAt: String(Date.now()),
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

/** The author's page paint: a light or a dark page, or `bare` for a page that sets no background
 *  (the render frame's canvas stays white). */
function pagePaint(options: { darkPage?: boolean; bare?: boolean }) {
  if (options.bare) return '';
  return `background:${options.darkPage ? '#181818' : '#fff'};color:${options.darkPage ? '#eee' : '#222'}`;
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
    bare?: boolean;
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
    source: `<style>body{margin:24px;font:16px/1.5 sans-serif;${pagePaint(options)}}tmt-question,tmt-proposal{display:block}</style><h1>Project notes</h1><p>Before the question.</p>${options.placement === 'nested' ? `<tmt-question data-id="${id(91)}">${markup}</tmt-question>` : markup}${options.placement === 'duplicate' ? markup : ''}<p id="after-question">The page continues here.</p>${options.mixed ? `<tmt-proposal data-id="${id(81)}"></tmt-proposal><p id="after-proposal">After both controls.</p>` : ''}`,
    threads: options.mixed ? current.threads : [],
  };
  emit();
}

/** Grouped authored markup uses the same parent parser and real quote/thread owners. */
export function questionGroup(
  options: {
    count?: number;
    layout?: 'centered' | 'nested';
    multiple?: boolean;
    darkPage?: boolean;
    bare?: boolean;
    longLabels?: boolean;
    lone?: boolean;
    agents?: string[];
    split?: boolean;
  } = {},
) {
  const count = options.count ?? 2;
  const questions = Array.from({ length: count }, (_, index) => {
    const kind = index === 0 ? 'single' : index === 1 ? 'short' : 'long';
    const label = options.longLabels
      ? `Question ${String(index + 1).padStart(2, '0')}: page review`
      : `Q${index + 1}`;
    return `<tmt-question data-id="${id(100 + index)}" data-kind="${kind}" data-agent="${options.agents?.[index] ?? 'Agent 1'}" data-label="${label}"><p data-question>${index === 0 ? 'Which heading should we use?' : index === 1 ? 'What should the introduction say?' : `What should section ${index + 1} explain?`}</p><p>Consider clarity and the intended reader.</p>${index === 0 ? '<span data-choice>Keep it</span><span data-choice>Change it</span>' : ''}</tmt-question>`;
  });
  const second = options.multiple
    ? `<tmt-questions data-id="${id(98)}"><p data-title>Second review</p>${questions.join('').replaceAll(id(100), id(110)).replaceAll(id(101), id(111)).replaceAll('Which heading should we use?', 'Which second heading should we use?').replaceAll('What should the introduction say?', 'What should the second introduction say?')}</tmt-questions>`
    : '';
  const block = options.split
    ? questions
        .map((question) =>
          question.replace('<tmt-question ', `<tmt-question data-group="${id(99)}" `),
        )
        .join('<p>A separate passage follows.</p>')
    : `<tmt-questions data-id="${id(99)}"><p data-title>Review the page</p>${questions.join('')}</tmt-questions>`;
  const main = `<h1>Project notes</h1><p id="container-prose">Before the question.</p>${block}${second}${options.lone ? `<tmt-question data-id="${id(120)}" data-kind="long"><p data-question>Any other feedback?</p></tmt-question>` : ''}<p id="after-question">The page continues here.</p>`;
  const content = main.replace(
    '<tmt-questions',
    '<div style="height:900px" aria-hidden="true"></div><tmt-questions',
  );
  const container =
    options.layout === 'nested'
      ? `<main style="max-width:1280px;margin:auto"><section id="author-container" style="max-width:620px;margin:auto">${content}</section></main><div style="height:1000px" aria-hidden="true"></div>`
      : options.layout === 'centered'
        ? `<main id="author-container" style="max-width:1280px;margin:auto">${content}</main><div style="height:1000px" aria-hidden="true"></div>`
        : main;
  current = {
    ...current,
    threads: [],
    asks: [],
    source: `<style>body{margin:24px;font:16px/1.5 sans-serif;${pagePaint(options)}}</style>${container}`,
  };
  emit();
}

/** A page whose answer block is the compact chip form, inline in running text. */
export function chipPage(options: { count?: 1 | 2; darkPage?: boolean; bare?: boolean } = {}) {
  const count = options.count ?? 2;
  const first = `<tmt-question data-id="${id(131)}" data-kind="single" data-agent="Agent 1"><p data-question>Which list version?</p><span data-choice>One column</span><span data-choice>Side categories</span></tmt-question>`;
  const second = `<tmt-question data-id="${id(132)}" data-kind="short" data-agent="Agent 1"><p data-question>Hide read pages?</p></tmt-question>`;
  const block =
    count === 1
      ? first.replace('data-kind="single"', 'data-kind="single" data-form="chip"')
      : `<tmt-questions data-id="${id(130)}" data-form="chip"><p data-title>Two list decisions</p>${first}${second}</tmt-questions>`;
  current = {
    ...current,
    threads: [],
    asks: [],
    source: `<style>body{margin:24px;font:16px/1.5 sans-serif;${pagePaint(options)}}</style><h1>Project notes</h1><div id="host-text">The list must filter and show each page's state. The two versions differ in where the categories live ${block} and the details follow in the next paragraph of this very long sentence so that it wraps on a narrow screen.</div><p id="after-question">The page continues here.</p>`,
  };
  emit();
}

/** A page of review sections: the parent parser, the real quote owner and the real status coordinator. */
export function review(
  options: {
    agent?: string;
    sections?: number;
    /** Put a question inside the first section; it closes the section when answered. */
    question?: boolean;
    /** Put a group of two questions inside the first section instead. */
    group?: boolean;
    placement?: 'inline' | 'duplicate' | 'nested';
    darkPage?: boolean;
    bare?: boolean;
    /** The page itself claims the section is closed and draws its own controls. */
    forged?: boolean;
  } = {},
) {
  const agent = options.agent ?? 'Agent 1';
  const section = (index: number) => {
    const inner =
      index === 0 && options.question
        ? `<tmt-question data-id="${id(210)}" data-kind="short" data-agent="${agent}"><p data-question>Which heading should we use?</p></tmt-question>`
        : index === 0 && options.group
          ? `<tmt-questions data-id="${id(209)}"><p data-title>Two questions</p><tmt-question data-id="${id(211)}" data-kind="short" data-agent="${agent}"><p data-question>First heading?</p></tmt-question><tmt-question data-id="${id(212)}" data-kind="short" data-agent="${agent}"><p data-question>Second heading?</p></tmt-question></tmt-questions>`
          : '';
    const forged = options.forged ? ' data-colab-review data-colab-review-closed="true"' : '';
    const claim = options.forged
      ? '<p>✓ Closed by the author</p><button type="button">Confirm and close</button>'
      : '';
    return `<tmt-review data-id="${id(200 + index)}" data-agent="${agent}"${forged}><h2 data-title>Section ${index + 1}</h2><p>Content of section ${index + 1}.</p>${claim}${inner}</tmt-review>`;
  };
  const sections = Array.from({ length: options.sections ?? 1 }, (_, index) => section(index));
  if (options.placement === 'duplicate') sections.push(section(0));
  if (options.placement === 'nested')
    sections[0] = sections[0].replace('</tmt-review>', `${section(7)}</tmt-review>`);
  current = {
    ...current,
    threads: [],
    asks: [],
    source: `<style>body{margin:24px;font:16px/1.5 sans-serif;${pagePaint(options)}}</style><h1>Project notes</h1><p>Before the sections.</p>${sections.join('')}<p id="after-sections">The page continues here.</p>`,
  };
  emit();
}

/** Another reader's thread on the same heading, as a sibling made by a concurrent first action. */
export function reviewSibling(options: {
  body: string;
  /** A person's Resolve from that reader, or an agent-labelled one. */
  resolved?: 'person' | 'agent';
  /** Orders before (low) or after (high) this device's writer ID. */
  writer?: number;
  /** Anchor the thread elsewhere than on the whole heading. */
  anchor?: QuoteSelector;
}) {
  const first = current.threads!.find((thread) => thread.anchor?.exact.startsWith('Section'))!;
  const writer = id(options.writer ?? 2),
    threadId = id(300 + (options.writer ?? 2)),
    messageId = id(400 + (options.writer ?? 2));
  const actionId = id(500 + (options.writer ?? 2));
  const sibling: ThreadView = {
    ...first,
    threadId,
    ref: { writer, id: threadId },
    ...(options.anchor ? { anchor: options.anchor } : {}),
    resolved: !!options.resolved,
    comments: [
      {
        ...first.comments[0],
        thread: { writer, id: threadId },
        messageId,
        ref: { writer, id: messageId },
        body: options.body,
        sequence: '1',
        deviceAuthor: true,
        deviceName: 'Other reader',
      },
    ],
    ...(options.resolved
      ? {
          status: {
            version: 1,
            kind: 'thread-status',
            spaceId: first.spaceId,
            pageId: first.pageId,
            epoch: first.epoch,
            senderDevice: writer,
            deviceName: 'Other reader',
            revision: '1',
            deleted: false,
            at: String(Date.now()),
            actionId,
            thread: { writer, id: threadId },
            previous: null,
            resolved: true,
            actor: options.resolved,
            agentName: options.resolved === 'agent' ? 'Atlas' : null,
            recipients: [],
            ref: { writer, id: actionId },
            depth: 1,
          },
        }
      : {}),
  };
  current = { ...current, threads: [...current.threads!, sibling] };
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

/** Admitted batch records for display/reload cases. The captured Ask uses the real validator. */
export function questionBatchAsk(state: PageAsk['state'] | 'replied') {
  current = {
    ...current,
    asks: answerAttempts.map(({ input, selection, operationId }) => ({
      thread: selection.thread,
      messageIds: selection.messageIds,
      operationId,
      writer: id(4),
      message: selection.comment,
      agent: input.destination.agent,
      agentName: input.destination.agentName,
      deviceName: 'You',
      machine: input.destination.machine,
      issuedAt: Date.now(),
      canTrack: true,
      state: state === 'replied' ? 'accepted' : state,
      // Match the accepted response supplied by this fixture's RemoteDouble.
      ...(state === 'accepted' || state === 'replied' ? { requestId: `req_${id(8)}` } : {}),
      ...(state === 'refused' ? { requestId: null, reason: 'REMOTE_SCOPE_DENIED' } : {}),
      ...(state === 'replied'
        ? { reply: `I read the answers: ${input.destination.agentName}.` }
        : {}),
    })),
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

/** Static admitted discussion projection for the annotation design matrix; no effect adapters. */
export function annotationLook() {
  conversation({
    surface: 'thread',
    state: 'replied',
    message: 'Could @Atlas check this at 390px? @Ben can review the copy.',
    reply: 'Checked the mobile layout. The message remains beside its source.',
  });
  const opening = current.threads![0];
  const quotes = [
    'Check the mobile layout before release.',
    'Make sure the Remote door restarts cleanly after the upgrade on every supported platform, including the slower CI runners.',
    'first range',
    'second range',
    'old decision text',
  ];
  current = {
    ...current,
    source: `<style>body{margin:40px;font:18px/1.65 sans-serif;max-width:650px;background:#fff;color:#343434}html[data-theme="dark"] body{background:#080808;color:#ddd}h2{font:13px monospace;letter-spacing:.12em;margin-top:30px}p{margin:14px 0}@media(max-width:640px){body{margin:24px}}</style><h2>01 / RELEASE CHECKLIST</h2><p id="selected"><strong>${quotes[0]}</strong></p><p>Keep the preview focused.</p><p id="wrapped">${quotes[1]}</p><p>Two notes next to each other: <span>${quotes[2]}</span> <span>${quotes[3]}</span>.</p><p>${quotes[4]}</p>`,
    threads: quotes.map((exact, index) => {
      const threadId = index === 0 ? opening.threadId : id(51 + index);
      const ref = { writer: opening.ref.writer, id: threadId };
      return {
        ...opening,
        threadId,
        ref,
        createdAt: String(index + 1),
        anchor: { exact, prefix: '', suffix: '' },
        resolved: index === 4,
        comments: [
          {
            ...opening.comments[0],
            thread: ref,
            messageId: index === 0 ? opening.comments[0].messageId : id(61 + index),
            ref: index === 0 ? opening.comments[0].ref : { writer: ref.writer, id: id(61 + index) },
          },
        ],
      };
    }),
  };
  emit();
}
export function annotationLookDelete() {
  current = {
    ...current,
    threads: current.threads!.map((thread, index) =>
      index === 0 ? { ...thread, deleted: true, anchor: null } : thread,
    ),
  };
  emit();
}

/** Bounded fixture gate for capturing the existing upload-progress and Send fence. */
export function pauseAttachmentUploads() {
  holdAttachmentUploads = true;
}
export function resumeAttachmentUploads() {
  holdAttachmentUploads = false;
  releaseAttachmentUpload?.();
  releaseAttachmentUpload = undefined;
}

/** Inert reading markup exercises the real page parser, renderer and person-status owner. */
export function reading(
  options: {
    count?: number;
    darkPage?: boolean;
    bare?: boolean;
    forged?: boolean;
    tall?: boolean;
  } = {},
) {
  const details = Array.from(
    { length: 24 },
    (_, paragraph) =>
      `<p>Decision detail ${paragraph + 1}: read the evidence, review the trade-offs, and confirm the next step with the team before continuing.</p>`,
  ).join('');
  current = {
    ...current,
    threads: [],
    asks: [],
    source: `<style>body{margin:24px;font:16px/1.5 sans-serif;${pagePaint(options)}}</style><h1>Release reading</h1><p>Read the decisions before the next step.</p>${Array.from({ length: options.count ?? 2 }, (_, index) => `<tmt-read${options.tall ? ' style="min-height:900px;display:block"' : ''} data-id="${id(240 + index)}" data-agent="Agent 1" data-why="Read before approving the release"${options.forged ? ' data-colab-reading-closed="true"' : ''}><h2 data-title>Decision ${index + 1}</h2><p>Content of reading ${index + 1}. The page remains readable after marking it read.</p>${options.tall ? details : ''}</tmt-read>`).join('')}<p>The page continues here.</p>`,
  };
  emit();
}

/** Already admitted notification outcomes; the UI never converts Resolve into delivery. */
export function readingDelivery(state: PageAsk['state'] | 'unavailable') {
  const thread = current.threads!.find((thread) => thread.status?.recipients.length)!;
  const status = thread.status!;
  const recipient = status.recipients[0];
  if (state === 'unavailable') {
    current = {
      ...current,
      asks: [],
      threads: current.threads!.map((row) =>
        row === thread
          ? {
              ...row,
              statuses: [status],
              notifications: [
                {
                  version: 1,
                  kind: 'thread-notification',
                  spaceId: thread.spaceId,
                  pageId: thread.pageId,
                  epoch: thread.epoch,
                  senderDevice: status.senderDevice,
                  revision: '1',
                  deleted: false,
                  deviceName: 'You',
                  at: status.at,
                  status: status.ref,
                  operationId: recipient.operationId,
                  reason: 'RECIPIENT_UNAVAILABLE',
                },
              ],
            }
          : row,
      ),
    };
  } else {
    current = {
      ...current,
      threads: current.threads!.map((row) =>
        row === thread ? { ...row, statuses: [status] } : row,
      ),
      asks: [
        {
          thread: thread.threadId,
          writer: status.ref.writer,
          operationId: recipient.operationId,
          machine: recipient.machine,
          agent: recipient.agent,
          agentName: recipient.agentName,
          messageIds: [],
          message: '',
          deviceName: 'You',
          issuedAt: Date.now(),
          canTrack: true,
          state,
        },
      ],
    };
  }
  emit();
}

/** A later authored requester rename cannot rewrite an admitted notification recipient. */
export function renameReadingRequester() {
  current = {
    ...current,
    source: current.source.replaceAll('data-agent="Agent 1"', 'data-agent="Agent 2"'),
  };
  emit();
}

/** Full source/provenance vector through the real layers and opaque renderer. */
export function pendingVector(vector: PendingVector) {
  current = { ...current, ...pendingVectorData(vector) };
  emit();
}
