/** Synthetic state-card and inline-notice hosts; no transport or real settings. */
import 'virtual:tokens.css';
import '@tmt/browser-ui/static.css';
import '../src/style.css';
import { createRoot } from 'react-dom/client';
import { BrowserNotice } from '@tmt/browser-ui/react';
import { TriangleAlert } from 'lucide-react';
import { NoticeCard } from '../src/notice-card.js';

export function mount() {
  const node = document.createElement('div');
  node.id = 'notice-layout-fixture';
  document.body.append(node);
  createRoot(node).render(
    <>
      <NoticeCard state="blocked" eyebrow="Colab" title="State card" testId="state-card">
        <p>State-card placement stays with this owner.</p>
      </NoticeCard>
      <div data-testid="inline-host">
        <BrowserNotice
          tone="waiting"
          announcement="none"
          stateLabel="Waiting"
          mark={<TriangleAlert />}
          title="Inline notice"
          body="This host supplies its own placement."
          headingLevel="none"
        />
      </div>
    </>,
  );
}
