import '@tmt/browser-ui/static.css';
import '../src/style.css';
import { createRoot } from 'react-dom/client';
import { BrowserIconAction } from '@tmt/browser-ui/react';
import { X } from 'lucide-react';
import { AgentChip, AgentList, agentSentence } from '../src/components/agent-chip.js';
import { text } from '../src/strings.js';

/** Every chip form and state in one column; the activity prop stands in for Remote's report. */
export function mount() {
  document.getElementById('root')?.setAttribute('hidden', '');
  const host = document.createElement('main');
  host.id = 'agent-chip-fixture';
  host.style.cssText = 'max-width:520px;margin:24px auto;padding:12px';
  document.body.append(host);
  createRoot(host).render(
    <>
      <p data-row="inline-idle">
        Waiting for <AgentChip name="Atlas" activity="idle" />
      </p>
      <p data-row="inline-working">
        Waiting for <AgentChip name="Atlas" activity="working" />
      </p>
      <p data-row="chip">
        <AgentChip name="Atlas" machine="My machine" variant="chip">
          <BrowserIconAction
            type="button"
            variant="text"
            label="Remove Atlas"
            icon={<X />}
            onActivate={() => undefined}
          />
        </AgentChip>
      </p>
      <p data-row="chip-working">
        <AgentChip name="Atlas" variant="chip" activity="working" />
      </p>
      <p data-row="chip-not-found">
        <AgentChip name="ghost" variant="chip" state="not-found" />
      </p>
      <p data-row="sentence">{agentSentence(text.askNoReply, 'Atlas')}</p>
      <p data-row="list">
        {text.threadNotNotified} <AgentList names={['alpha', 'beta']} />
      </p>
    </>,
  );
}
