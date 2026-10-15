/** Presentation only: no Auth, provider, authority claim or real sign-in. Reader crypto has its own vectors. */
import 'virtual:tokens.css';
import '@tmt/browser-ui/static.css';
import '../src/style.css';
import { createRoot } from 'react-dom/client';
import { HostedApp, HostedRequestNotice } from '../src/hosted-app.js';
import type { HostedEntryState } from '../src/hosted-entry.js';

const states: Record<string, HostedEntryState> = {
  'sign-in': { phase: 'sign-in', hasLink: true },
  'sign-in-viewer': { phase: 'sign-in', hasLink: true, role: 'viewer' },
  'sign-in-commenter': { phase: 'sign-in', hasLink: true, role: 'commenter' },
  'sign-in-editor': { phase: 'sign-in', hasLink: true, role: 'editor' },
  verifying: { phase: 'loading' },
  recovering: { phase: 'recovering' },
  paused: { phase: 'paused' },
  ready: {
    phase: 'ready',
    view: {
      title: 'Shared design notes',
      source:
        '<main><h1>Verified shared page</h1><p>This page can be read here. Editing stays with its owner.</p></main>',
    },
    budget: { reads: 3501, writes: 1, day: 1, state: 'warn' },
  },
  reshare: { phase: 'failed', failure: { code: 'COLAB_HOSTED_RESHARE_REQUIRED' } },
  budget: {
    phase: 'failed',
    failure: { code: 'REMOTE_BUDGET_EXHAUSTED', resetAtMs: Date.parse('2026-11-02T08:00:00Z') },
  },
  'rate-limited': { phase: 'failed', failure: { code: 'REMOTE_RATE_LIMITED' } },
  denied: { phase: 'failed', failure: { code: 'REMOTE_SCOPE_DENIED' } },
  missing: { phase: 'failed', failure: { code: 'COLAB_HOSTED_NOT_IN_CLOUD' } },
};
const selected = new URL(location.href).searchParams.get('state') ?? 'sign-in';
createRoot(document.getElementById('root')!).render(
  selected === 'request-sent' ||
    selected === 'request-refused' ||
    selected === 'request-unconfirmed' ? (
    <main>
      <HostedRequestNotice
        outcome={
          selected === 'request-sent'
            ? 'sent'
            : selected === 'request-refused'
              ? 'refused'
              : 'unconfirmed'
        }
      />
    </main>
  ) : (
    <HostedApp state={states[selected] ?? states['sign-in']} signIn={() => {}} />
  ),
);
