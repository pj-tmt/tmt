/** Chat composer and row-menu doubles: prefill sources, a blocked Enter, IME, and the ⋯ menu. */
import { createRoot, type Root } from 'react-dom/client';
import { AnnotationInput } from '../src/annotation-input.js';
import type { AskBinding } from '../src/ask-panel.js';
import { ConversationTurn } from '../src/components/conversation-turn.js';
import { ActionMenu } from '../src/components/action-menu.js';
import type { ThreadBinding } from '../src/thread-store.js';
import { destination, id } from './ask-fixtures.js';

let root: Root | undefined;
let createChats = 0;
let selected: string[] = [];

function host() {
  root?.unmount();
  document.getElementById('chat-fixture')?.remove();
  document.getElementById('root')?.setAttribute('hidden', '');
  const main = document.createElement('main');
  main.id = 'chat-fixture';
  document.body.append(main);
  createChats = 0;
  selected = [];
  return main;
}

export function mountComposer(options: { agents: string[]; selected?: number; replier?: string }) {
  const base = destination();
  const agents = options.agents.map((agentName, index) => ({
    ...base,
    agent: id(20 + index),
    agentName,
  }));
  const binding: AskBinding = {
    async destinations() {
      return agents;
    },
    async prepare() {
      throw new Error('Not used');
    },
    async recheck() {},
    async abandon() {},
  };
  const discussion = {
    deviceId: id(4),
    async createChat() {
      createChats++;
      throw new Error('Counted');
    },
  } as unknown as ThreadBinding;
  root = createRoot(host());
  root.render(
    <AnnotationInput
      chat
      binding={binding}
      discussion={discussion}
      anchor={null}
      asks={[]}
      title="Chat page"
      initialEdit={{
        value: '',
        recipient: options.selected === undefined ? undefined : agents[options.selected],
      }}
      replier={(() => {
        const matches = agents.filter((agent) => agent.agentName === options.replier);
        return matches.length === 1
          ? { machine: matches[0].machine, agent: matches[0].agent }
          : undefined;
      })()}
      blocked={false}
      cancel={() => {}}
      committed={() => {}}
    />,
  );
}

export function mountMenu(options: { tight?: boolean; spaceAbove?: boolean } = {}) {
  root = createRoot(host());
  const row = (
    <ConversationTurn
      role="user"
      layout="thread"
      author="You"
      at={Date.now()}
      className="comment"
      data-testid="row"
      style={{ width: 360, padding: 16 }}
      actions={
        <>
          <span role="status">replied</span>
          <ActionMenu
            label="Message actions"
            items={[
              { key: 'edit', label: 'Edit' },
              { key: 'delete', label: 'Delete' },
            ]}
            onSelect={(key) => selected.push(key)}
          />
        </>
      }
    >
      <p>First line of a message.</p>
      <button type="button">After</button>
    </ConversationTurn>
  );
  // The small viewport either has room above the row or needs an in-viewport menu.
  root.render(
    options.tight ? (
      <div
        data-testid="menu-viewport"
        style={{ height: options.spaceAbove ? 240 : 120, overflowY: 'auto', marginTop: 200 }}
      >
        {options.spaceAbove && <div style={{ height: 120 }} />}
        {row}
      </div>
    ) : (
      row
    ),
  );
}

export function proof() {
  return { createChats, selected };
}
