/** Component fixture: real conversation/action/controller/store, doubled discussion and Remote.
 * Production sync, durable comments and recipient effects are verified by native acceptance. */
import { createRoot, type Root } from 'react-dom/client';
import { useEffect, useState } from 'react';
import { ChatPanel } from '../src/chat-panel.js';
import { ThreadWindow } from '../src/thread-panel.js';
import type { AskBinding, PageAsk } from '../src/ask-panel.js';
import type { CommentContext, ThreadBinding } from '../src/thread-store.js';
import type { ThreadView } from '../src/thread-records.js';
import { record } from '../src/storage.js';
import { fixtureAttempt } from './ask-browser-attempt.js';
import { destination, id, selection } from './ask-fixtures.js';

type Mode =
  | 'prepare'
  | 'unadopted'
  | 'refused'
  | 'refused-again'
  | 'uncertain'
  | 'unknown'
  | 'held';
type Saved = {
  thread: ThreadView;
  records: PageAsk[];
  sends: { agentId: string; operationId: string }[];
  preparations: number;
  writes: number;
  inputs: Parameters<AskBinding['prepare']>[0][];
};
const savedKey = 'test:ask-again-view';
let root: Root | undefined;
let state: Saved;
let release: (() => void) | undefined;
let park = false;
let directory = 'ready';
let replace: (() => void) | undefined;
let readonly = false;
let reverseRetry = false;
export const agents = [
  { ...destination(), agentName: 'alpha' },
  { ...destination(), agent: id(9), agentName: 'beta' },
];
export function setPark(value: boolean) {
  park = value;
}
export function isParked() {
  return release !== undefined;
}
export function releasePreparation() {
  release?.();
}
export function setDirectory(value: string) {
  directory = value;
}
export function replaceBinding() {
  replace?.();
}
export function proof() {
  return structuredClone(state);
}
export async function mount(
  options: {
    reverse?: boolean;
    surface?: 'chat' | 'thread';
    mode?: Mode;
    restore?: boolean;
    readonly?: boolean;
  } = {},
) {
  root?.unmount();
  document.getElementById('again-fixture')?.remove();
  document.getElementById('root')?.setAttribute('hidden', '');
  const host = document.createElement('main');
  host.id = 'again-fixture';
  host.className = 'comments-panel';
  document.body.append(host);
  const surface = options.surface ?? 'chat';
  const mode = options.mode ?? 'refused';
  readonly = !!options.readonly;
  reverseRetry = !!options.reverse;
  directory = 'ready';
  park = false;
  release = undefined;
  state = options.restore
    ? (await record<Saved>(savedKey))!
    : {
        thread: {
          version: 1,
          kind: 'thread',
          spaceId: selection().space,
          pageId: id(1),
          epoch: '1',
          senderDevice: id(4),
          deviceName: 'You',
          revision: '1',
          deleted: false,
          at: String(Date.now()),
          threadId: surface === 'chat' ? id(4) : id(2),
          ref: { writer: id(4), id: surface === 'chat' ? id(4) : id(2) },
          anchor: surface === 'chat' ? null : { exact: 'Selected text', prefix: '', suffix: '' },
          resolved: false,
          comments: [],
        },
        records: [],
        sends: [],
        preparations: 0,
        writes: 0,
        inputs: [],
      };
  if (!options.restore) await record(`test:ask-own:${selection().space}:${id(1)}:${id(4)}`, {});
  location.hash = `space=${selection().space}&path=${encodeURIComponent(`/pages/${id(1)}`)}`;
  function Fixture() {
    const [, render] = useState(0);
    async function refresh() {
      await record(savedKey, state);
      render((value) => value + 1);
    }
    async function write(body: string): Promise<CommentContext> {
      state.writes++;
      const ref = { writer: id(4), id: crypto.randomUUID() };
      state.thread.comments.push({
        ...state.thread,
        kind: 'comment',
        messageId: ref.id,
        ref,
        thread: state.thread.ref,
        body,
        at: String(Date.now()),
      });
      await refresh();
      return { thread: state.thread.ref, message: ref, threadRevision: '1', messageRevision: '1' };
    }
    const unused = async () => {
      throw new Error('Not used');
    };
    const discussion: ThreadBinding = {
      deviceId: readonly ? id(77) : id(4),
      create: write,
      createChat: write,
      reply: async (_ref, body) => write(body),
      edit: unused,
      deleteComment: unused,
      setStatus: unused,
      notificationFailed: unused,
      updateThread: unused,
    };
    const makeBinding = (): AskBinding => ({
      async destinations() {
        if (directory === 'fail') throw new Error('Directory unavailable');
        return directory === 'renamed' ? [{ ...agents[0], agent: id(78) }, agents[1]] : agents;
      },
      async prepare(input) {
        state.preparations++;
        state.inputs.push(structuredClone(input));
        const first = state.preparations === 1;
        if (first && mode === 'prepare') throw new Error('Preparation unavailable');
        const context = input.context!;
        const comment = state.thread.comments.find((value) => value.ref.id === context.message.id);
        if (!comment || comment.deleted || comment.revision !== context.messageRevision)
          throw new Error('Stale message');
        const captured = {
          ...selection(),
          ...input,
          thread: context.thread.id,
          messageIds: [context.message.id],
          quote: state.thread.anchor?.exact ?? '',
          comment: comment.body,
        };
        const f = await fixtureAttempt(captured, input.destination, {
          retryOf: input.retryOf,
          mode:
            input.destination.agent === agents[0].agent && mode === 'uncertain'
              ? 'throw'
              : first && mode === 'held'
                ? 'held'
                : 'accepted',
        });
        if (park)
          await new Promise<void>((resolve) => {
            release = resolve;
          });
        const refusal =
          input.destination.agent === agents[0].agent &&
          ((first && ['refused', 'refused-again'].includes(mode)) ||
            (input.retryOf !== undefined && mode === 'refused-again'));
        if (refusal)
          f.remote.send = async (value) => {
            f.remote.sends.push(value);
            return {
              state: 'refused',
              operationId: value.operationId,
              reason: 'REMOTE_RATE_LIMITED',
            };
          };
        return {
          ...f.attempt,
          send: async () => {
            if (first && mode === 'unadopted') return { state: 'failed', adopted: false };
            if (first && mode === 'unknown') return { state: 'uncertain' };
            const result = await f.attempt.send();
            const view = f.attempt.preview.view;
            state.sends.push(...f.remote.sends);
            const row: PageAsk = {
              operationId: view.operationId,
              writer: id(4),
              thread: context.thread.id,
              messageIds: [context.message.id],
              machine: input.destination.machine,
              agent: input.destination.agent,
              agentName: input.destination.agentName,
              deviceName: 'You',
              issuedAt: Date.now(),
              message: view.message,
              state: result.state as PageAsk['state'],
              canTrack: !readonly,
              requestId: result.state === 'accepted' ? `req_${id(8)}` : null,
              reason: refusal ? 'REMOTE_RATE_LIMITED' : undefined,
            };
            if (reverseRetry) {
              if (input.retryOf === undefined) state.records.unshift(row);
              else state.records.splice(1, 0, row);
            } else state.records.push(row);
            await refresh();
            return { ...result, adopted: true };
          },
        };
      },
      recheck: async () => {},
      abandon: async () => {},
    });
    const [binding, setBinding] = useState<AskBinding>(makeBinding);
    useEffect(() => {
      replace = () => setBinding(makeBinding());
      return () => {
        replace = undefined;
      };
    }, []);
    return surface === 'chat' ? (
      <ChatPanel
        threads={[state.thread]}
        asks={state.records}
        binding={binding}
        discussion={discussion}
        title="Retry fixture"
        blocked={false}
        close={() => {}}
      />
    ) : (
      <ThreadWindow
        thread={state.thread}
        asks={state.records}
        ask={binding}
        binding={discussion}
        title="Retry fixture"
        attached
        anchorsChecked
        selection={null}
        blocked={false}
        close={() => {}}
      />
    );
  }
  root = createRoot(host);
  root.render(<Fixture />);
}
