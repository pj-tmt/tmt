/** Controlled admitted-parent fixture; no real Remote credentials or dispatch. */
import 'virtual:tokens.css';
import '@tmt/browser-ui/static.css';
import '../src/style.css';
import { createRoot } from 'react-dom/client';
import { useEffect, useState } from 'react';
import { RouterProvider } from '@tanstack/react-router';
import { createAppRouter } from '../src/router.js';
import { localTransport, type PageBinding, type PageView } from '../src/transport.js';
import type { AskBinding } from '../src/ask-panel.js';
import type { AgentDirectoryObservation } from '../src/live-ask.js';
import type { ThreadBinding } from '../src/thread-store.js';
import { AgentStatusPanel } from '../src/agent-status-panel.js';
import { ChatPanel } from '../src/chat-panel.js';
import { destination, id } from './ask-fixtures.js';

let mode:
  | 'ready'
  | 'empty'
  | 'directory-failed'
  | 'session-ended'
  | 'session-evicted'
  | 'channel-unavailable'
  | 'scope-refused'
  | 'pending' = 'ready';
let checks = 0,
  prepares = 0,
  writes = 0,
  ledgerActions = 0,
  destinationReads = 0;
let publish: ((view: PageView) => void) | undefined;
let fail: ((error: Error) => void) | undefined;
const pending: ((observation: AgentDirectoryObservation) => void)[] = [];
const source =
  '<!doctype html><main><h1>Storage discussion</h1><p>Keep this page and draft in place while checking your agents.</p></main>';
const agents = ['active', 'offline', 'unknown'].map((presence, index) => ({
  ...destination(),
  agent: id(6 + index),
  agentName: index < 2 ? 'Same name' : 'Unobserved agent',
  machineName: 'Studio Mac',
  presence: presence as 'active' | 'offline' | 'unknown',
}));
function ready(replacement = false): AgentDirectoryObservation {
  return {
    kind: 'ready',
    checkedAt: Date.now(),
    destinations:
      mode === 'empty'
        ? []
        : agents.map((agent) => ({
            ...agent,
            agentName: replacement ? `New ${agent.agentName}` : agent.agentName,
          })),
  };
}
function client(replacement = false): AskBinding {
  return {
    async observeDestinations() {
      checks++;
      if (mode === 'pending' && !replacement)
        return new Promise((resolve) => pending.push(resolve));
      if (mode === 'directory-failed')
        return { kind: 'unavailable', phase: 'directory', failure: 'unavailable' };
      if (mode === 'session-ended')
        return {
          kind: 'unavailable',
          phase: 'directory',
          failure: 'ended',
          code: 'REMOTE_SESSION_ENDED',
        };
      if (mode === 'channel-unavailable')
        return {
          kind: 'unavailable',
          phase: 'directory',
          failure: 'unavailable',
          code: 'REMOTE_SEQUENCE_UNAVAILABLE',
        };
      if (mode === 'session-evicted')
        return {
          kind: 'unavailable',
          phase: 'session',
          failure: 'evicted',
          code: 'REMOTE_SESSION_EVICTED',
        };
      if (mode === 'scope-refused')
        return {
          kind: 'unavailable',
          phase: 'directory',
          failure: 'refused',
          code: 'REMOTE_SCOPE_DENIED',
        };
      return ready(replacement);
    },
    async destinations() {
      destinationReads++;
      return agents;
    },
    async prepare() {
      prepares++;
      throw new Error('Unexpected status dispatch');
    },
    async recheck() {
      ledgerActions++;
    },
    async abandon() {
      ledgerActions++;
    },
  };
}
let ask: AskBinding | undefined = client();
async function unexpectedDiscussion(): Promise<never> {
  writes++;
  throw new Error('Unexpected discussion publication');
}
const discussion: ThreadBinding = {
  deviceId: id(4),
  create: unexpectedDiscussion,
  createChat: unexpectedDiscussion,
  reply: unexpectedDiscussion,
  edit: unexpectedDiscussion,
  deleteComment: unexpectedDiscussion,
  updateThread: unexpectedDiscussion,
  setStatus: unexpectedDiscussion,
  notificationFailed: unexpectedDiscussion,
};
const binding: PageBinding = {
  discussion,
  get ask() {
    return ask;
  },
  subscribe(value, error) {
    publish = value;
    fail = error;
    return () => {
      publish = undefined;
      fail = undefined;
    };
  },
  async edit() {
    writes++;
  },
  async export() {
    throw new Error('Unexpected export');
  },
  close() {},
};
const transport = localTransport('Review space', [
  { id: 'status-page', title: 'Storage discussion', source, sharing: 'private', binding },
]);
const root = createRoot(document.getElementById('root')!);
root.render(
  <RouterProvider router={createAppRouter({ ...transport, backendName: 'Studio Mac' })} />,
);
location.hash = '/pages/status-page';
export function setMode(value: typeof mode) {
  mode = value;
}
export function replaceClient() {
  ask = client(true);
  publish?.({ title: 'Storage discussion', source });
}
export function resolvePrevious() {
  for (const resolve of pending.splice(0)) resolve(ready());
}
export function failPage() {
  fail?.(new Error('Fixture page connection unavailable'));
}
export function proof() {
  return { checks, prepares, writes, ledgerActions, destinationReads };
}

export function disableClient() {
  ask = undefined;
  publish?.({ title: 'Storage discussion', source });
}

let changeAdmission: ((admitted: boolean) => void) | undefined;
function AdmissionProbe() {
  const [admitted, setAdmitted] = useState(true);
  useEffect(() => {
    changeAdmission = setAdmitted;
    return () => {
      changeAdmission = undefined;
    };
  }, []);
  return (
    <main>
      <AgentStatusPanel open binding={ask} page="status-page" admitted={admitted} />
      <ChatPanel
        threads={[]}
        asks={[]}
        binding={ask}
        discussion={discussion}
        title="Storage discussion"
        blocked={!admitted}
        close={() => {}}
      />
    </main>
  );
}
/** Same parent inputs as the drawer; admission alone changes while the client,
 * page and mounted Chat composer remain identical. */
export function mountAdmissionProbe() {
  root.render(<AdmissionProbe />);
}
export function setAdmission(admitted: boolean) {
  changeAdmission?.(admitted);
}
export function resolveOldest() {
  pending.shift()?.(ready());
}
export function refuseNewest() {
  pending.pop()?.({
    kind: 'unavailable',
    phase: 'directory',
    failure: 'refused',
    code: 'REMOTE_REFUSED',
  });
}
