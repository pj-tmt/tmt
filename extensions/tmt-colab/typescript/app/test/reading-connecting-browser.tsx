/** A reading band's action row with the real components, to watch the cue beside a disabled Mark read. */
import '@tmt/browser-ui/static.css';
import '../src/style.css';
import { BrowserAction } from '@tmt/browser-ui/react';
import { createRoot, type Root } from 'react-dom/client';
import { AgentChip } from '../src/components/agent-chip.js';
import { NeedsYouBar } from '../src/components/needs-you-bar.js';
import { ReadingConnecting } from '../src/reading-connecting.js';

let root: Root | undefined;
export function mount(waiting: boolean) {
  const host = document.getElementById('root')!;
  root ??= createRoot(host);
  root.render(
    <section className="proposal-inline reading-inline" style={{ position: 'static' }}>
      <NeedsYouBar
        tone="needs"
        word="Please read"
        who={<AgentChip name="Agent 1" />}
        why="Read before approving the release"
        actions={
          <>
            <BrowserAction
              type="button"
              variant="text"
              label="Mark read"
              disabled={waiting}
              onActivate={() => undefined}
            />
            <ReadingConnecting waiting={waiting} />
            <BrowserAction
              type="button"
              variant="text"
              label="Next ↓"
              disabled
              onActivate={() => undefined}
            />
          </>
        }
      />
      <p>The passage below the band stays where it is.</p>
    </section>,
  );
}
