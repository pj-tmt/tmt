/** Header snapshot fixture: no real door, pairing, provider or transport is opened. */
import { createRoot, type Root } from 'react-dom/client';
import { RouterProvider } from '@tanstack/react-router';
import { createAppRouter } from '../src/router.js';
import { localTransport } from '../src/transport.js';
import { RemotePresence, type RemoteDevicePage } from '../src/remote-presence.js';

let root: Root | undefined;
let owner: RemotePresence | undefined;
let calls = 0;
let failure = false;
let release: (() => void) | undefined;
let gate: Promise<void> | undefined;
let count = 3;
const atMs = new Date('2026-10-16T10:42:00').getTime();
const id = 'a19c0460-1111-4111-8111-000000000001';
export function reads() {
  return calls;
}
export function refuse(value = true) {
  failure = value;
}
export function hold() {
  gate = new Promise<void>((done) => {
    release = done;
  });
}
export function resume() {
  release?.();
  gate = undefined;
}
export function setCount(value: number) {
  count = value;
}
export function visibility(visible: boolean) {
  Object.defineProperty(document, 'visibilityState', {
    configurable: true,
    value: visible ? 'visible' : 'hidden',
  });
  document.dispatchEvent(new Event('visibilitychange'));
}
export async function mount(
  input: {
    count?: number;
    hidden?: boolean;
    home?: boolean;
    unconfirmed?: boolean;
    pending?: boolean;
  } = {},
) {
  root?.unmount();
  await owner?.close();
  document.getElementById('root')!.hidden = true;
  let host = document.getElementById('remote-header-fixture');
  if (!host) {
    host = document.createElement('div');
    host.id = 'remote-header-fixture';
    document.body.append(host);
  }
  calls = 0;
  failure = input.unconfirmed === true;
  count = input.count ?? 3;
  gate = undefined;
  if (input.pending) hold();
  const generation = {};
  owner = new RemotePresence(
    () => ({
      generation,
      devices: async (): Promise<RemoteDevicePage> => {
        calls++;
        await gate;
        if (failure) throw new Error('Refused');
        return {
          nextCursor: null,
          devices: [
            {
              clientId: 'cli',
              name: 'not the CLI label',
              kind: 'cli',
              thisBrowser: false,
              liveSessionCount: 0,
              lastActivityAtMs: null,
              expiresAtMs: null,
              revoked: false,
            },
            {
              clientId: 'addon',
              name: 'not the add-on label',
              kind: 'addon',
              thisBrowser: false,
              liveSessionCount: Math.max(0, count - 1),
              lastActivityAtMs: atMs - 60000,
              expiresAtMs: null,
              revoked: false,
            },
            {
              clientId: 'self',
              name: 'not the current label',
              kind: 'browser',
              thisBrowser: true,
              liveSessionCount: Math.min(1, count),
              lastActivityAtMs: atMs,
              expiresAtMs: null,
              revoked: false,
            },
          ],
        };
      },
    }),
    new URL('/settings', location.href).href,
    () => atMs,
  );
  const transport = localTransport('Pages', [
    {
      id,
      title: 'Remote header',
      sharing: 'private',
      source:
        '<main><h1>Remote header</h1><p>Read the page while its header shows a door snapshot.</p></main>',
    },
  ]);
  const router = createAppRouter({
    ...transport,
    backendName: 'Studio Mac',
    remotePresence: input.hidden ? undefined : owner,
  });
  await router.navigate({ to: input.home ? '/' : '/pages/$pageId', params: { pageId: id } });
  root = createRoot(host);
  root.render(<RouterProvider router={router} />);
}
export async function unmount() {
  root?.unmount();
  root = undefined;
  await owner?.close();
}
