/** Presentation-only scale fixture; no native store, transport or dispatch is opened. */
import { createRoot, type Root } from 'react-dom/client';
import { browserUiClasses as ui } from '@tmt/browser-ui/static';
import { MessageSquare } from 'lucide-react';
import { ThreadWindow } from '../src/thread-panel.js';
import { ChatPanel } from '../src/chat-panel.js';
import { ConversationTurn } from '../src/components/conversation-turn.js';
import type { ThreadView } from '../src/thread-records.js';
import type { ThreadBinding } from '../src/thread-store.js';
import type { AskBinding, PageAsk } from '../src/ask-panel.js';
import { destination, id, selection } from './ask-fixtures.js';

let root: Root | undefined;
const unused = async () => {
  throw new Error('Scale fixture must not write or dispatch');
};
const discussion: ThreadBinding = {
  deviceId: id(4),
  create: unused,
  createChat: unused,
  reply: unused,
  edit: unused,
  deleteComment: unused,
  setStatus: unused,
  notificationFailed: unused,
  updateThread: unused,
};
const binding: AskBinding = {
  destinations: async () => [{ ...destination(), agentName: 'astra', presence: 'active' }],
  prepare: unused,
  recheck: unused,
  sendNow: unused,
  abandon: unused,
};

export function mount(surface: 'annotation' | 'chat') {
  root?.unmount();
  document.getElementById('scale-fixture')?.remove();
  document.getElementById('root')!.hidden = true;
  const host = document.createElement('div');
  host.id = 'scale-fixture';
  host.className =
    surface === 'annotation' ? 'annotation-popover comments-panel' : 'comments-panel';
  // Only position/viewport containment is supplied; production components own all scale styles.
  host.style.cssText =
    'position:fixed;top:70px;left:12px;width:min(520px,calc(100vw - 24px));max-height:calc(100dvh - 82px);display:flex;flex-direction:column;';
  document.body.append(host);
  const at = 1791946800000;
  const threadId = surface === 'chat' ? id(4) : id(2);
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
    createdAt: String(at),
    at: String(at),
    threadId,
    ref: { writer: id(4), id: threadId },
    anchor:
      surface === 'chat'
        ? null
        : { exact: 'A clear place to read and share work.', prefix: '', suffix: '' },
    resolved: false,
    comments: [],
  };
  thread.comments = [
    {
      ...thread,
      kind: 'comment',
      messageId: id(3),
      ref: { writer: id(4), id: id(3) },
      thread: thread.ref,
      body: 'Please check the layout and keep the response beside this text.',
      at: String(at),
    },
  ];
  const asks: PageAsk[] = [
    {
      thread: threadId,
      messageIds: [id(3)],
      operationId: id(8),
      writer: id(4),
      message: thread.comments[0]!.body,
      agent: id(6),
      agentName: 'astra',
      deviceName: 'Browser',
      issuedAt: at,
      machine: id(5),
      state: 'accepted',
      canTrack: false,
      reply: 'The message, mention and composer should use the same readable body scale.',
    },
  ];
  const initialEdit = {
    value: '@astra Please check the spacing.',
    edited: true,
    mentions: [{ key: { machine: id(5), agent: id(6) }, range: { start: 0, end: 6 } }],
  };
  root = createRoot(host);
  root.render(
    <>
      {/* Same presentation as the router's selection button; intentionally no action handler. */}
      <button className={`selection-ask ${ui.action}`} data-testid="scale-annotate">
        <MessageSquare aria-hidden />
        Annotate
      </button>
      {surface === 'annotation' ? (
        <ThreadWindow
          number={1}
          thread={thread}
          attached={false}
          anchorsChecked
          selection={null}
          binding={discussion}
          ask={binding}
          asks={asks}
          title="Scale audit"
          layout="anchored"
          initialEdit={initialEdit}
          observationUnavailable
          blocked={false}
          close={() => {}}
        />
      ) : (
        <ChatPanel
          threads={[thread]}
          asks={asks}
          discussion={discussion}
          binding={binding}
          title="Scale audit"
          initialEdit={initialEdit}
          observationUnavailable
          blocked={false}
          close={() => {}}
        />
      )}
    </>,
  );
}

/** Question-card replies also use the turn outside a conversation window. */
export function mountInline() {
  root?.unmount();
  const host = document.getElementById('scale-fixture')!;
  root = createRoot(host);
  root.render(
    <ConversationTurn role="agent" author="astra" at={1791946800000}>
      Inline reply outside the conversation window.
    </ConversationTurn>,
  );
}
