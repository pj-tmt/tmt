/** Component-only send/keyboard double; durable writer and Remote proof lives in acceptance. */
import { createRoot, type Root } from 'react-dom/client';
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
let remote: RemoteDouble | undefined;
export function mount(mode: 'accepted' | 'held' | 'throw' | 'prepare-failure' | 'multi' = 'held') {
  root?.unmount();
  document.getElementById('annotation-fixture')?.remove();
  document.getElementById('root')?.setAttribute('hidden', '');
  const host = document.createElement('main');
  host.id = 'annotation-fixture';
  document.body.append(host);
  writes = preparations = closes = commits = 0;
  remote = undefined;
  location.hash = `space=${selection().space}&path=${encodeURIComponent(`/pages/${id(1)}`)}`;
  const target = destination();
  const ref = { writer: id(4), id: id(2) };
  const discussion: ThreadBinding = {
    deviceId: id(4),
    async create() {
      writes++;
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
        const result = await fixtureAttempt({ ...selection(), ...input }, target, {
          mode: mode === 'multi' ? 'held' : mode,
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
            publisher={target.agentName}
            blocked={false}
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
