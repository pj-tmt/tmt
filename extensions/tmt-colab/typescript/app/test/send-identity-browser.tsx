/** The real chat panel with a send-attribution source (#4255): what the page tells the person once. */
import { createRoot, type Root } from 'react-dom/client';
import { ChatPanel } from '../src/chat-panel.js';
import type { AskBinding } from '../src/ask-panel.js';
import type { SendIdentity } from '../src/ask-remote.js';
import { SendIdentityNotice } from '../src/send-identity-notice.js';
import { destination } from './ask-fixtures.js';

let root: Root | undefined;
let notice: SendIdentityNotice | undefined;

/** Mount the panel; `identity` is what Remote said about the first send, absent for none. */
export function mountChat(identity?: SendIdentity) {
  root?.unmount();
  document.getElementById('identity-fixture')?.remove();
  document.getElementById('root')?.setAttribute('hidden', '');
  const main = document.createElement('main');
  main.id = 'identity-fixture';
  document.body.append(main);
  notice = new SendIdentityNotice();
  const binding: AskBinding = {
    identity: notice,
    async destinations() {
      return [destination()];
    },
    async prepare() {
      throw new Error('Not used');
    },
    async recheck() {},
    async abandon() {},
  };
  root = createRoot(main);
  root.render(
    <ChatPanel
      threads={[]}
      asks={[]}
      binding={binding}
      title="Chat page"
      blocked={false}
      close={() => {}}
    />,
  );
  if (identity) notice.show(identity);
}
/** A later send in the same page session: the line is not shown again. */
export function sendAgain(identity: SendIdentity) {
  notice?.show(identity);
}
