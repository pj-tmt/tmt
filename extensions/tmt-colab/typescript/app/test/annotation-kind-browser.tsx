import '@tmt/browser-ui/static.css';
import '../src/style.css';
import '../src/components/message-composer.css';
import { createRoot } from 'react-dom/client';
import { AgentChip } from '../src/components/agent-chip.js';

/** Explicit cosmetic kind fixtures only; no editor keys, directory or recipient effects. */
export function mount() {
  document.getElementById('root')?.setAttribute('hidden', '');
  const host = document.createElement('main');
  host.id = 'annotation-kind-fixture';
  host.style.cssText = 'max-width:720px;margin:24px auto;padding:12px';
  document.body.append(host);
  createRoot(host).render(
    <>
      {['Composer', 'Sent message', 'Mention list'].map((surface) => (
        <section key={surface} className="comment-thread" aria-label={surface}>
          <header className="conversation-window-bar">
            <h2 className="conversation-title">{surface}</h2>
          </header>
          <div className="conversation-composer">
            <p className="rich-input-field">
              Could <AgentChip name="@reviewer" /> check 390px?
              <br />
              <AgentChip name="@Ben" variant="chip" /> review copy?
            </p>
          </div>
        </section>
      ))}
    </>,
  );
}
