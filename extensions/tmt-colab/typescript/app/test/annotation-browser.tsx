/** Component-only send/keyboard double; durable writer and Remote proof lives in acceptance. */
import { createRoot, type Root } from 'react-dom/client';
import { ThreadWindow } from '../src/thread-panel.js';
import { MessageComposer } from '../src/components/message-composer.js';
import type { ThreadView } from '../src/thread-records.js';
import { useState } from 'react';
import { AnnotationInput } from '../src/annotation-input.js';
import { AskPanel, type AskBinding, type PageAsk } from '../src/ask-panel.js';
import type { ThreadBinding } from '../src/thread-store.js';
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
  remote = undefined;
  draft = '';
  captured = [];
  location.hash = `space=${selection().space}&path=${encodeURIComponent(`/pages/${id(1)}`)}`;
  const target = destination();
  const ref = { writer: id(4), id: id(2) };
  const discussion: ThreadBinding = {
    deviceId: id(4),
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
  function Fixture() {
    const [records, setRecords] = useState<PageAsk[]>([]);
    const [cancelled, setCancelled] = useState(false);
    const binding: AskBinding = {
      async destinations() {
        if (mode === 'discovery-failure') throw new Error('Discovery refused');
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
    };
    return (
      <>
        {!cancelled && (
          <AnnotationInput
            binding={binding}
            discussion={discussion}
            anchor={{ exact: 'Selected text', prefix: '', suffix: '' }}
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

export function editingProof() {
  return { draft, captured };
}

let statusCalls = 0;
let finishStatus: ((failed: boolean) => void) | undefined;
/** Presentation-only writer binding double, with no status store or dispatch adapter. */
export function mountWindow(mode = 'ready') {
  root?.unmount();
  document.getElementById('annotation-fixture')?.remove();
  document.getElementById('root')?.setAttribute('hidden', '');
  const host = document.createElement('main');
  host.id = 'annotation-fixture';
  document.body.append(host);
  statusCalls = closes = 0;
  finishStatus = undefined;
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
    resolved: false,
    comments: [],
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
        blocked={mode === 'blocked'}
        close={() => {
          closes++;
        }}
        composer={
          <MessageComposer label="Window draft" edit={edit} onChange={setEdit} disabled={false} />
        }
        binding={{
          deviceId: mode === 'readonly' ? id(7) : id(4),
          create: unused,
          createChat: unused,
          reply: unused,
          edit: unused,
          deleteComment: unused,
          updateThread: async (_ref, _revision, change) => {
            statusCalls++;
            await new Promise<void>((resolve, reject) => {
              finishStatus = (failed) => {
                if (failed) reject(new Error('Unavailable'));
                else {
                  setCurrent((previous) => ({ ...previous, ...change }));
                  resolve();
                }
              };
            });
          },
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
