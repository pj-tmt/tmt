/** Component-only send/keyboard double; durable writer and Remote proof lives in acceptance. */
import { createRoot, type Root } from 'react-dom/client';
import { ThreadPanel, ThreadWindow } from '../src/thread-panel.js';
import { MessageComposer } from '../src/components/message-composer.js';
import type { ThreadView } from '../src/thread-records.js';
import { useEffect, useState } from 'react';
import { AnnotationInput } from '../src/annotation-input.js';
import { AskPanel, type AskBinding, type PageAsk } from '../src/ask-panel.js';
import { ReadRefusedError } from '../src/ask-remote.js';
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
let captured: unknown[] = [];
let remote: RemoteDouble | undefined;
/** What the next directory read does: answer, fail, or wait until released. */
let directoryMode: 'ok' | 'fail' | 'refuse' | 'park' = 'ok';
let directoryReads = 0;
let parkedReads: { resolve(): void; reject(error: Error): void }[] = [];
let reconnect: (() => void) | undefined;
/** Which composer surface the next mount shows: a new annotation, a reply, or Chat. */
let surface: 'annotation' | 'reply' | 'chat' = 'annotation';
export function setSurface(next: 'annotation' | 'reply' | 'chat') {
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
export function mount(
  mode:
    | 'accepted'
    | 'held'
    | 'throw'
    | 'prepare-failure'
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
  directoryMode = mode === 'discovery-failure' ? 'fail' : 'ok';
  directoryReads = 0;
  parkedReads = [];
  remote = undefined;
  draft = '';
  captured = [];
  location.hash = `space=${selection().space}&path=${encodeURIComponent(`/pages/${id(1)}`)}`;
  const target = destination();
  const ref = { writer: id(4), id: id(2) };
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
      captured.push({ body, anchor });
      if (mode === 'write-failure') throw new Error('Write refused');
      return {
        thread: ref,
        threadRevision: '1',
        message: { writer: id(4), id: id(3) },
        messageRevision: '1',
      };
    },
    async createChat() {
      throw new Error('Not used');
    },
    async reply() {
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
    at: '1',
    threadId: id(2),
    ref,
    anchor: { exact: 'Selected text', prefix: '', suffix: '' },
    resolved: false,
    comments: [],
  };
  function Fixture() {
    const [records, setRecords] = useState<PageAsk[]>([]);
    const [cancelled, setCancelled] = useState(false);
    const makeBinding = (): AskBinding => ({
      async destinations() {
        directoryReads++;
        if (directoryMode === 'fail') throw new Error('Discovery refused');
        // Remote's signed refusal, as the Remote SDK raises it for a read.
        if (directoryMode === 'refuse') throw new ReadRefusedError('REMOTE_STATE_UNAVAILABLE');
        if (directoryMode === 'park')
          await new Promise<void>((resolve, reject) => parkedReads.push({ resolve, reject }));
        return mode === 'multi'
          ? [
              target,
              { ...target, agent: id(8), agentName: 'Disabled agent', presence: 'offline' },
              { ...target, agent: id(9), agentName: 'Other agent' },
            ]
          : [target];
      },
      async prepare(input) {
        preparations++;
        if (mode === 'prepare-failure') throw new Error('Preparation refused');
        captured.push(input);
        const result = await fixtureAttempt({ ...selection(), ...input }, input.destination, {
          mode:
            mode === 'multi' || mode === 'discovery-failure' || mode === 'write-failure'
              ? 'held'
              : mode,
          operationId: crypto.randomUUID(),
        });
        remote = result.remote;
        return {
          ...result.attempt,
          async send() {
            const state = await result.attempt.send();
            const record: PageAsk = {
              thread: ref.id,
              messageIds: [id(3)],
              operationId: result.attempt.preview.view.operationId,
              writer: id(4),
              message: result.attempt.preview.view.message,
              deliveredMessage: result.attempt.preview.view.deliveredMessage,
              agent: target.agent,
              agentName: target.agentName,
              deviceName: target.deviceName,
              issuedAt: Date.now(),
              machine: target.machine,
              state: state.state as PageAsk['state'],
              canTrack: true,
            };
            setRecords((previous) => (mode === 'accepted' ? [...previous, record] : [record]));
            return state;
          },
        };
      },
      async recheck() {},
      async abandon() {},
    });
    const [binding, setBinding] = useState<AskBinding>(makeBinding);
    useEffect(() => {
      reconnect = () => setBinding(makeBinding());
    });
    return (
      <>
        {!cancelled && (
          <AnnotationInput
            binding={binding}
            discussion={discussion}
            chat={surface === 'chat'}
            thread={surface === 'reply' ? replyThread : undefined}
            anchor={surface === 'chat' ? null : { exact: 'Selected text', prefix: '', suffix: '' }}
            asks={records}
            title="Annotated page"
            blocked={false}
            onDraft={(value) => {
              draft = value;
            }}
            cancel={() => {
              closes++;
              setCancelled(true);
            }}
            committed={() => {
              commits++;
            }}
          />
        )}
        <AskPanel inline records={records} binding={binding} blocked={false} />
      </>
    );
  }
  root = createRoot(host);
  root.render(<Fixture />);
}
export function proof() {
  return { writes, preparations, closes, commits, sends: remote?.sends ?? [] };
}
export function directoryProof() {
  return { reads: directoryReads, parked: parkedReads.length };
}

export function editingProof() {
  return { draft, captured };
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
    deleted: false,
    at: '1',
    threadId: id(2),
    ref: { writer: id(4), id: id(2) },
    anchor: { exact: 'Frozen original quote', prefix: '', suffix: '' },
    resolved: mode === 'capture-agent' || mode === 'capture-person',
    comments: [],
  };
  if (capture)
    thread.comments = [
      {
        ...thread,
        kind: 'comment',
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
    if (mode === 'capture-compose')
      return (
        <ThreadPanel
          threads={[]}
          resolved={[]}
          anchorsChecked
          selection={null}
          binding={binding}
          asks={[]}
          title="Annotated page"
          blocked={false}
          active={null}
          select={() => {}}
        />
      );
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
