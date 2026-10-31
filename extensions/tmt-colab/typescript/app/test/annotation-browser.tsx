import { ConversationPresentation } from '../src/components/conversation-presentation.js';
/** Component-only send/keyboard double; durable writer and Remote proof lives in acceptance. */
import { createRoot, type Root } from 'react-dom/client';
import { ThreadWindow } from '../src/thread-panel.js';
import { ChatPanel } from '../src/chat-panel.js';
import { MessageComposer } from '../src/components/message-composer.js';
import type { ThreadView } from '../src/thread-records.js';
import { useEffect, useState } from 'react';
import type { AgentDestination } from '../src/live-ask.js';
import { CreatorDefaultProvider, type CreatorTarget } from '../src/creator-default.js';
import type { ComposerEdit } from '../src/components/message-composer-edit.js';
import { AnnotationInput } from '../src/annotation-input.js';
import { AskPanel, type AskBinding, type PageAsk } from '../src/ask-panel.js';
import { ReadRefusedError, SessionEndedError, SessionEvictedError } from '../src/ask-remote.js';
import { OwnRecordConflict, OwnAppendRefused } from '../src/own-publication.js';
import { getNearestEditorFromDOMNode, $getRoot, $isElementNode } from 'lexical';
import { MessageMentionNode } from '../src/components/message-composer-document.js';
import type { ThreadBinding } from '../src/thread-store.js';
import { projectThreadStatus } from '../src/thread-status-view.js';
import { destination, id, selection, type RemoteDouble } from './ask-fixtures.js';
import { fixtureAttempt } from './ask-browser-attempt.js';
let root: Root | undefined;
let writes = 0;
let preparations = 0;
let closes = 0;
let commits = 0;
let draft = '';
let draftEdit: ComposerEdit | undefined;
let talk = true;
export function setTalk(value: boolean) {
  talk = value;
}
let captured: unknown[] = [];
/** What the next directory read does: answer, fail, or wait until released. */
let directoryMode: 'ok' | 'fail' | 'refuse' | 'park' = 'ok';
let directoryReads = 0;
let parkedReads: { resolve(): void; reject(error: Error): void }[] = [];
let reconnect: (() => void) | undefined;
let changeConnection: ((phase: 'connected' | 'reconnecting' | 'ended') => void) | undefined;
export function setConnection(phase: 'connected' | 'reconnecting' | 'ended') {
  changeConnection?.(phase);
}
/** Which composer surface the next mount shows: a new annotation, a reply, or Chat. */
let creator: CreatorTarget | undefined;
let changeCreator: ((value: CreatorTarget | undefined) => void) | undefined;
let firstFrame: { chips: number; value: string } | undefined;
export function firstPaintProof() {
  return firstFrame;
}
export function setCreatorMetadata(value: CreatorTarget | undefined) {
  creator = value;
  changeCreator?.(value);
}
let customAgents: AgentDestination[] | undefined;
let allSends: RemoteDouble['sends'] = [];
let preferenceScope: string | undefined;
let initialEdit: ComposerEdit | undefined;
let pendingWrite: (() => void) | undefined;
let waitForWrite: Promise<void> | undefined;
export function holdWrite() {
  waitForWrite = new Promise<void>((resolve) => {
    pendingWrite = resolve;
  });
}
export function releaseWrite() {
  pendingWrite?.();
  waitForWrite = undefined;
  pendingWrite = undefined;
}
export function setPreferenceScope(scope: string | undefined) {
  preferenceScope = scope;
}
export function setInitialEdit(edit: ComposerEdit | undefined) {
  initialEdit = edit;
}
export function setCreator(value: boolean) {
  setCreatorMetadata(
    value
      ? {
          machineId: destination().machine,
          agentId: destination().agent,
          label:
            customAgents?.find((agent) => agent.agent === destination().agent)?.agentName ??
            destination().agentName,
        }
      : undefined,
  );
}
export function setAgents(value: AgentDestination[]) {
  customAgents = value;
}
/** Mount the reply or Chat composer inside its real host, which shows sent messages at once. */
let echoHost = false;
export function setEchoHost(value: boolean) {
  echoHost = value;
}
let publicationGate: Promise<void> | undefined;
let releasePublication: (() => void) | undefined;
export function holdAskPublication() {
  publicationGate = new Promise<void>((resolve) => {
    releasePublication = resolve;
  });
}
export function releaseAskPublication() {
  releasePublication?.();
  publicationGate = undefined;
  releasePublication = undefined;
}
let publishVerified: (() => void) | undefined;
/** The verified view republishes with the last recorded message, as the live projection would. */
export function publishRecorded() {
  publishVerified?.();
}
let surface: 'annotation' | 'reply' | 'chat' | 'linked' = 'annotation';
export function setSurface(next: 'annotation' | 'reply' | 'chat' | 'linked') {
  surface = next;
}
export function setDirectory(mode: 'ok' | 'fail' | 'refuse' | 'park') {
  directoryMode = mode;
}
/** Settles every parked read, as a slow Remote answer or a late failure would. */
export function releaseDirectory(outcome: 'ok' | 'fail' = 'ok') {
  const parked = parkedReads;
  parkedReads = [];
  for (const read of parked) {
    if (outcome === 'ok') read.resolve();
    else read.reject(new Error('Late'));
  }
}
/** A Reconnect replaces the Ask binding the composer holds, mid-draft. */
export function reconnectAsk() {
  reconnect?.();
}
let writeFailure: Error | undefined;
export function setWriteFailure(
  kind: 'conflict' | 'sequence' | 'ended' | 'evicted' | 'unavailable',
) {
  writeFailure =
    kind === 'conflict'
      ? new OwnRecordConflict()
      : kind === 'sequence'
        ? new OwnAppendRefused('CONFLICT')
        : kind === 'ended'
          ? new SessionEndedError('REMOTE_SESSION_ENDED')
          : kind === 'evicted'
            ? new SessionEvictedError(2)
            : new SessionEndedError('REMOTE_SEQUENCE_UNAVAILABLE');
}
/** Model background token normalization: metadata changes, visible draft text does not. */
export function normalizeMention() {
  const field = document.querySelector<HTMLElement>('#annotation-fixture [role="combobox"]');
  if (!field) throw new Error('Missing fixture field');
  const editor = getNearestEditorFromDOMNode(field);
  if (!editor) throw new Error('Missing fixture editor');
  editor.update(
    () => {
      for (const paragraph of $getRoot().getChildren()) {
        if (!$isElementNode(paragraph)) continue;
        for (const node of paragraph.getChildren())
          if (node instanceof MessageMentionNode)
            node.getWritable().__defaultCreator = !node.__defaultCreator;
      }
    },
    { discrete: true },
  );
}
export function mount(
  mode:
    | 'accepted'
    | 'held'
    | 'throw'
    | 'prepare-failure'
    | 'partial-preparation'
    | 'partial-refusal'
    | 'core-refusal'
    | 'no-adoption'
    | 'unknown-adoption'
    | 'multi'
    | 'discovery-failure'
    | 'write-failure' = 'held',
) {
  root?.unmount();
  document.getElementById('annotation-fixture')?.remove();
  document.getElementById('root')?.setAttribute('hidden', '');
  const host = document.createElement('main');
  host.id = 'annotation-fixture';
  document.body.append(host);
  writes = preparations = closes = commits = 0;
  writeFailure = undefined;
  if (mode === 'discovery-failure') directoryMode = 'fail';
  directoryReads = 0;
  parkedReads = [];
  draft = '';
  captured = [];
  allSends = [];
  location.hash = `space=${selection().space}&path=${encodeURIComponent(`/pages/${id(1)}`)}`;
  const target = destination();
  const ref = { writer: id(4), id: id(2) };
  const chatRef = { writer: id(4), id: id(4) };
  const discussion: ThreadBinding = {
    deviceId: id(4),
    async setStatus() {
      throw new Error('Not used');
    },
    async notificationFailed() {
      throw new Error('Not used');
    },
    async create(body, anchor) {
      writes++;
      await waitForWrite;
      captured.push({ body, anchor });
      if (mode === 'write-failure') throw writeFailure ?? new Error('Write refused');
      return {
        thread: ref,
        threadRevision: '1',
        message: { writer: id(4), id: id(3) },
        messageRevision: '1',
      };
    },
    async createChat(body) {
      return { ...(await this.create(body, null)), thread: chatRef };
    },
    async reply(thread, body) {
      return { ...(await this.create(body, null)), thread };
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
  const replyThread: ThreadView = {
    version: 1,
    kind: 'thread',
    spaceId: selection().space,
    pageId: id(1),
    epoch: '1',
    senderDevice: id(4),
    deviceName: 'Browser',
    revision: '1',
    deleted: false,
    createdAt: '1',
    at: '1',
    threadId: id(2),
    ref,
    anchor: { exact: 'Selected text', prefix: '', suffix: '' },
    resolved: false,
    comments: [],
  };
  firstFrame = undefined;
  const firstPaint = new MutationObserver(() => {
    const field = host.querySelector<HTMLElement>('[role="combobox"]');
    if (!field) return;
    firstPaint.disconnect();
    requestAnimationFrame(() => {
      // Read token labels without the action's hidden tooltip text.
      const content = field.cloneNode(true) as HTMLElement;
      content.querySelectorAll('.message-mention-host').forEach((host) => {
        host.textContent = host.querySelector('.agent-chip-name')?.textContent ?? '';
      });
      firstFrame = {
        chips: field.querySelectorAll('.agent-chip[data-variant="chip"]').length,
        value: content.textContent,
      };
    });
  });
  firstPaint.observe(host, { childList: true, subtree: true });
  function Fixture() {
    const [currentCreator, setCurrentCreator] = useState(creator);
    useEffect(() => {
      changeCreator = setCurrentCreator;
      return () => {
        changeCreator = undefined;
      };
    }, []);
    const [records, setRecords] = useState<PageAsk[]>([]);
    const [cancelled, setCancelled] = useState(false);
    const [phase, setPhase] = useState<'connected' | 'reconnecting' | 'ended'>('connected');
    useEffect(() => {
      changeConnection = setPhase;
      return () => {
        changeConnection = undefined;
      };
    }, []);
    const connectionProps = {
      connectionPresentation:
        phase === 'connected'
          ? undefined
          : {
              phase,
              body:
                phase === 'reconnecting'
                  ? 'Reconnecting…'
                  : 'Access to this page has ended. Pair again.',
            },
      recoveryRequired: phase === 'reconnecting',
    };
    const makeBinding = (): AskBinding => ({
      async destinations() {
        directoryReads++;
        if (directoryMode === 'fail') throw new Error('Discovery refused');
        // Remote's signed refusal, as the Remote SDK raises it for a read.
        if (directoryMode === 'refuse') throw new ReadRefusedError('REMOTE_STATE_UNAVAILABLE');
        if (directoryMode === 'park')
          await new Promise<void>((resolve, reject) => parkedReads.push({ resolve, reject }));
        return (
          customAgents ??
          (mode === 'multi'
            ? [
                target,
                { ...target, agent: id(8), agentName: 'Disabled agent', presence: 'offline' },
                { ...target, agent: id(9), agentName: 'Other agent' },
              ]
            : [target])
        );
      },
      async prepare(input) {
        preparations++;
        if (mode === 'prepare-failure' || (mode === 'partial-preparation' && preparations === 1))
          throw new Error('Preparation refused');
        captured.push(input);
        const refused = ['partial-refusal', 'core-refusal'].includes(mode) && preparations === 1;
        const refusal = mode === 'core-refusal' ? 'REMOTE_CORE_UNAVAILABLE' : 'REMOTE_RATE_LIMITED';
        const result = await fixtureAttempt(
          {
            ...selection(),
            ...input,
            thread: input.context?.thread.id ?? selection().thread,
            messageIds: input.context ? [input.context.message.id] : [],
          },
          input.destination,
          {
            mode:
              mode === 'partial-preparation' ||
              mode === 'partial-refusal' ||
              mode === 'core-refusal' ||
              mode === 'no-adoption' ||
              mode === 'unknown-adoption'
                ? 'accepted'
                : mode === 'multi' || mode === 'discovery-failure' || mode === 'write-failure'
                  ? 'held'
                  : mode,
            operationId: crypto.randomUUID(),
            async published(views) {
              setRecords(
                views.map((view) => ({
                  thread: view.intent.thread,
                  messageIds: view.intent.messageIds,
                  operationId: view.intent.operationId,
                  writer: view.writer,
                  message: view.intent.message,
                  agent: view.intent.agent,
                  agentName: view.agentName,
                  deviceName: view.deviceName,
                  issuedAt: view.intent.issuedAt,
                  machine: view.intent.machine,
                  state: view.state,
                  reason: view.reason,
                  canTrack: true,
                })),
              );
              await publicationGate;
            },
          },
        );
        if (refused)
          result.remote.send = async (input) => {
            result.remote.sends.push(input);
            return {
              state: 'refused',
              operationId: input.operationId,
              reason: refusal,
            };
          };
        return {
          ...result.attempt,
          async send() {
            if (mode === 'unknown-adoption') return { state: 'uncertain' as const };
            if (mode === 'no-adoption') return { state: 'failed' as const, adopted: false };
            const state = await result.attempt.send();
            allSends.push(...result.remote.sends);
            const record: PageAsk = {
              thread: input.context?.thread.id ?? ref.id,
              messageIds: [id(3)],
              operationId: result.attempt.preview.view.operationId,
              writer: id(4),
              message: result.attempt.preview.view.message,
              deliveredMessage: result.attempt.preview.view.deliveredMessage,
              agent: input.destination.agent,
              agentName: input.destination.agentName,
              deviceName: target.deviceName,
              issuedAt: Date.now(),
              machine: target.machine,
              state: state.state as PageAsk['state'],
              canTrack: true,
              ...(refused ? { reason: refusal } : {}),
            };
            setRecords((previous) => [
              ...previous.filter((row) => row.operationId !== record.operationId),
              record,
            ]);
            return state;
          },
        };
      },
      async recheck() {},
      async sendNow() {},
      async abandon() {},
    });
    const [binding, setBinding] = useState<AskBinding>(makeBinding);
    useEffect(() => {
      reconnect = () => setBinding(makeBinding());
    });
    const [view, setView] = useState<ThreadView>(() =>
      surface === 'chat'
        ? { ...replyThread, threadId: id(4), ref: chatRef, anchor: null }
        : replyThread,
    );
    useEffect(() => {
      publishVerified = () =>
        setView((previous) => ({
          ...previous,
          comments: [
            ...previous.comments,
            {
              ...previous,
              kind: 'comment',
              deleted: false,
              messageId: id(3),
              ref: { writer: id(4), id: id(3) },
              thread: previous.ref,
              body: (
                [...captured]
                  .reverse()
                  .find((value) => !!value && typeof value === 'object' && 'body' in value) as {
                  body: string;
                }
              ).body,
              at: String(Date.now()),
              sequence: String(previous.comments.length + 1),
            },
          ],
        }));
      return () => {
        publishVerified = undefined;
      };
    }, []);
    if (echoHost && surface === 'chat')
      return (
        <ChatPanel
          threads={[view]}
          asks={records}
          binding={talk ? binding : undefined}
          discussion={discussion}
          title="Annotated page"
          blocked={phase !== 'connected'}
          onDraft={(edit) => {
            draft = edit.value;
            draftEdit = edit;
          }}
          close={() => {
            closes++;
          }}
        />
      );
    if (echoHost && surface === 'reply')
      return (
        <ThreadWindow
          thread={view}
          attached
          anchorsChecked
          selection={null}
          asks={records}
          title="Annotated page"
          blocked={phase !== 'connected'}
          close={() => {
            closes++;
          }}
          binding={discussion}
          ask={talk ? binding : undefined}
          onDraft={(edit) => {
            draft = edit.value;
            draftEdit = edit;
          }}
        />
      );
    const composer = !cancelled && (
      <AnnotationInput
        {...connectionProps}
        creationRecipient={currentCreator}
        initialEdit={initialEdit}
        binding={talk ? binding : undefined}
        discussion={discussion}
        thread={surface === 'reply' ? replyThread : undefined}
        anchor={surface === 'chat' ? null : { exact: 'Selected text', prefix: '', suffix: '' }}
        asks={records}
        title="Annotated page"
        blocked={phase !== 'connected'}
        onDraft={(value, edit) => {
          draft = value;
          draftEdit = edit;
        }}
        cancel={() => {
          closes++;
          setCancelled(true);
        }}
        committed={() => {
          commits++;
        }}
      />
    );
    return (
      <ConversationPresentation value={surface === 'chat' ? 'chat' : 'thread'}>
        {composer}
        {surface === 'linked' && composer}
        <AskPanel inline records={records} binding={binding} blocked={false} />
      </ConversationPresentation>
    );
  }
  root = createRoot(host);
  root.render(
    preferenceScope ? (
      <CreatorDefaultProvider scope={preferenceScope}>
        <Fixture />
      </CreatorDefaultProvider>
    ) : (
      <Fixture />
    ),
  );
}
export function proof() {
  return { writes, preparations, closes, commits, sends: allSends };
}
export function directoryProof() {
  return { reads: directoryReads, parked: parkedReads.length };
}

export function editingProof() {
  return { draft, edit: draftEdit, captured };
}

let statusCalls = 0;
let finishStatus: ((failed: boolean) => void) | undefined;
let finishWrite: ((failed: boolean) => void) | undefined;
/** Presentation-only writer binding double, with no status store or dispatch adapter. */
export function mountWindow(mode = 'ready') {
  root?.unmount();
  document.getElementById('annotation-fixture')?.remove();
  document.getElementById('root')?.setAttribute('hidden', '');
  const capture = mode.startsWith('capture-');
  const host = document.createElement(capture ? 'div' : 'main');
  host.id = 'annotation-fixture';
  if (capture) {
    host.className = 'annotation-popover comments-panel';
    host.style.top = '96px';
    host.style.left = '12px';
  }
  document.body.append(host);
  statusCalls = closes = 0;
  finishStatus = undefined;
  finishWrite = undefined;
  writes = 0;
  captured = [];
  const unused = async () => {
    throw new Error('Not used');
  };
  const thread: ThreadView = {
    version: 1,
    kind: 'thread',
    spaceId: selection().space,
    pageId: id(1),
    epoch: '1',
    senderDevice: id(4),
    deviceName: 'Browser',
    revision: '1',
    deleted: mode === 'capture-deleted',
    createdAt: '1',
    at: '1',
    threadId: mode === 'capture-chat' ? id(4) : id(2),
    ref: { writer: id(4), id: mode === 'capture-chat' ? id(4) : id(2) },
    anchor:
      mode === 'capture-chat' || mode === 'capture-deleted'
        ? null
        : { exact: 'Frozen original quote', prefix: '', suffix: '' },
    resolved: mode === 'capture-agent' || mode === 'capture-person',
    comments: [],
  };
  if (capture)
    thread.comments = [
      {
        ...thread,
        kind: 'comment',
        deleted: false,
        messageId: id(3),
        ref: { writer: id(4), id: id(3) },
        thread: thread.ref,
        body: 'Keep the conversation beside the selected page text.',
        at: String(Date.now()),
      },
    ];
  async function write(body: string) {
    writes++;
    captured.push(body);
    await new Promise<void>((resolve, reject) => {
      finishWrite = (failed) => (failed ? reject(new Error('Unavailable')) : resolve());
    });
  }
  const binding: ThreadBinding = {
    deviceId: mode === 'readonly' || mode === 'other-writer' ? id(7) : id(4),
    create: async (body) => {
      await write(body);
      return {
        thread: thread.ref,
        threadRevision: '1',
        message: { writer: id(4), id: id(3) },
        messageRevision: '1',
      };
    },
    createChat: unused,
    reply: unused,
    edit: async (_ref, _revision, body) => write(body),
    deleteComment: unused,
    setStatus: unused,
    notificationFailed: unused,
    // Status goes through onStatusChange; the thread binding must never see it.
    updateThread: unused,
  };
  function WindowFixture() {
    const [current, setCurrent] = useState(thread);
    const [edit, setEdit] = useState({ value: 'Unsent draft' });
    return (
      <ThreadWindow
        thread={current}
        attached
        anchorsChecked
        selection={null}
        asks={[]}
        title="Annotated page"
        layout={capture ? 'anchored' : 'panel'}
        blocked={mode === 'blocked' || mode === 'capture-disabled'}
        close={() => {
          closes++;
        }}
        initialEdit={capture ? edit : undefined}
        composer={
          capture ? undefined : (
            <MessageComposer label="Window draft" edit={edit} onChange={setEdit} disabled={false} />
          )
        }
        binding={binding}
        status={{
          ...projectThreadStatus(current, undefined, mode !== 'readonly'),
          actor: mode === 'capture-agent' ? 'agent' : 'person',
          actorName: mode === 'capture-agent' ? 'Release coordination assistant' : 'Browser',
        }}
        onStatusChange={async (resolved) => {
          statusCalls++;
          await new Promise<void>((resolve, reject) => {
            finishStatus = (failed) => {
              if (failed) reject(new Error('Unavailable'));
              else {
                setCurrent((previous) => ({ ...previous, resolved }));
                resolve();
              }
            };
          });
          return { changed: true };
        }}
      />
    );
  }
  root = createRoot(host);
  root.render(<WindowFixture />);
}
export function finishWindowStatus(mode?: string) {
  finishStatus?.(mode === 'failed');
  finishStatus = undefined;
}
export function windowProof() {
  return { statusCalls, closes };
}

export function finishWindowWrite(mode?: string) {
  finishWrite?.(mode === 'failed');
  finishWrite = undefined;
}
export function windowWriteProof() {
  return { writes, captured };
}
