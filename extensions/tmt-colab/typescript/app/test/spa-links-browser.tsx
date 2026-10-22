/** Browser-only fixture for the one-TMT-app conditions: Colab as a route under `/colab/`. It starts the
 * real router and the real module entry over the local transports; no door and no native state. */
import type { ReactNode } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { RouterProvider } from '@tanstack/react-router';
import type { ShellSlot } from '@tmt/browser-ui/shell-host';
import { hostInShell } from '../src/entry.js';
import { mount as mountColab } from '../src/colab-module.js';
import { MarkdownText } from '../src/components/markdown-text.js';
import { createAppRouter, shellLocation } from '../src/router.js';
import { previewTransport } from '../src/local-pages.js';
import { ColabModuleBoundary } from '../src/colab-module-boundary.js';
import { localTransport } from '../src/transport.js';

export const space = '10000000-0000-4000-8000-000000000001';
export const pageId = '12345678-0000-4000-8000-000000000001';
export const readingId = '10000000-0000-4000-8000-000000000401';
export const anchorId = 'digest-matrix';

const source =
  '<h1>Original content</h1>' +
  `<tmt-read data-id="${readingId}" data-agent="Agent 1" data-why="Read before deciding">` +
  '<h2 data-title>Recommended passage</h2><p>The passage.</p></tmt-read>' +
  `<div style="height:3000px"></div><h2 id="${anchorId}">Digest matrix</h2><p>The matrix.</p>` +
  '<div style="height:1500px"></div>';

let root: Root | undefined;

/** Opens `url` (path, query and fragment, as a link would) in the router the TMT app hosts at
 * `/colab/`: the location is rewritten by the module entry exactly as it is on a real load. A
 * `body` is also shown as a comment body (`#spa-body`), so its links click inside the router;
 * `pageSource` replaces the page's own source. */
export function mountAt(url: string, body?: string, pageSource = source) {
  root?.unmount();
  document.getElementById('spa-fixture')?.remove();
  document.getElementById('root')?.setAttribute('hidden', '');
  const host = document.createElement('div');
  host.id = 'spa-fixture';
  document.body.append(host);
  history.replaceState(null, '', url);
  hostInShell(null);
  shellLocation();
  const transport = localTransport('Colab', [
    { id: pageId, title: 'Original proposal', sharing: 'private' as const, source: pageSource },
  ]);
  const spaceHome = transport.spaceHome;
  transport.spaceHome = async () => ({
    ...(await spaceHome()),
    pageIds: [{ pageId, deleted: false }],
  });
  root = createRoot(host);
  const Wrap = ({ children }: { children: ReactNode }) => (
    <>
      {children}
      {body !== undefined && (
        <div id="spa-body">
          <MarkdownText value={body} />
        </div>
      )}
    </>
  );
  root.render(<RouterProvider router={createAppRouter(transport)} InnerWrap={Wrap} />);
}

/** What the module started, recorded so a test can see it released. */
interface Started {
  sockets: WebSocket[];
  intervals: Set<number>;
  workers: { terminated: boolean }[];
  channels: { closed: boolean }[];
}
declare global {
  interface Window {
    __started?: Started;
  }
}

/** Records every socket, interval, worker and broadcast channel created from here on. */
export function trackStarted() {
  const started: Started = { sockets: [], intervals: new Set(), workers: [], channels: [] };
  window.__started = started;
  const NativeSocket = window.WebSocket;
  window.WebSocket = class extends NativeSocket {
    constructor(...args: ConstructorParameters<typeof WebSocket>) {
      super(...args);
      started.sockets.push(this);
    }
  };
  const setInterval = window.setInterval.bind(window);
  const clearInterval = window.clearInterval.bind(window);
  window.setInterval = ((...args: Parameters<typeof window.setInterval>) => {
    const id = setInterval(...args);
    started.intervals.add(id);
    return id;
  }) as typeof window.setInterval;
  window.clearInterval = ((id?: number) => {
    if (id !== undefined) started.intervals.delete(id);
    clearInterval(id);
  }) as typeof window.clearInterval;
  const NativeWorker = window.Worker;
  window.Worker = class extends NativeWorker {
    constructor(...args: ConstructorParameters<typeof Worker>) {
      super(...args);
      const entry = { terminated: false };
      started.workers.push(entry);
      const terminate = this.terminate.bind(this);
      this.terminate = () => {
        entry.terminated = true;
        terminate();
      };
    }
  };
  const NativeChannel = window.BroadcastChannel;
  window.BroadcastChannel = class extends NativeChannel {
    constructor(name: string) {
      super(name);
      const entry = { closed: false };
      started.channels.push(entry);
      const close = this.close.bind(this);
      this.close = () => {
        entry.closed = true;
        close();
      };
    }
  };
}

let unmountColab: (() => void) | undefined;
let slotHost: HTMLElement | undefined;
let restoreCrash: (() => void) | undefined;

/** Poisons the local preview so the Home route throws while it renders: a page whose title is not
 * text. The router's own error view takes over; the host's rail and header slot must stay. */
function crashHome(): () => void {
  const home = previewTransport.spaceHome;
  previewTransport.spaceHome = async () => ({
    title: 'Crash',
    pages: [{ id: 'welcome', title: 5 as unknown as string, sharing: 'private' as const }],
  });
  return () => {
    previewTransport.spaceHome = home;
  };
}

/** The slot the host gives a route: a bar for the header and a content area. */
function newSlot(url: string): ShellSlot {
  document.getElementById('root')?.setAttribute('hidden', '');
  slotHost?.remove();
  slotHost = document.createElement('div');
  slotHost.id = 'spa-slot';
  const bar = document.createElement('div');
  const main = document.createElement('div');
  slotHost.append(bar, main);
  document.body.append(slotHost);
  history.replaceState(null, '', url);
  return {
    main,
    bar,
    basePath: '/colab/',
    answer: () => undefined,
    setMenu: () => {},
    setTitle: () => {},
    navigate: () => {},
    describe: () => {},
    described: () => {},
  };
}

/** Mounts Colab the way the TMT app does: a slot, no composed shell (so the local preview).
 * `crash` makes the Home route throw while it renders. */
export async function mountModule(url: string, crash = false) {
  restoreCrash?.();
  restoreCrash = crash ? crashHome() : undefined;
  unmountColab = await mountColab(newSlot(url));
}

let boundaryBroken = false;

/** Whatever the boundary guards stops throwing: the next Try again renders it. */
export function healBoundary() {
  boundaryBroken = false;
}

/** The module boundary alone, over a child that throws until `healBoundary` (React retries a
 * failed render once, so a count would not hold). */
export function mountBoundary() {
  const slot = newSlot('/colab/');
  const root = createRoot(slot.main);
  boundaryBroken = true;
  const Child = () => {
    if (boundaryBroken) throw new Error('Colab test failure');
    return <p>Recovered</p>;
  };
  const silence = console.error;
  console.error = () => {};
  root.render(
    <ColabModuleBoundary slot={slot}>
      <Child />
    </ColabModuleBoundary>,
  );
  unmountColab = () => {
    root.unmount();
    console.error = silence;
  };
}

/** The host releases the route before the next module mounts. */
export function unmountModule() {
  unmountColab?.();
  unmountColab = undefined;
  restoreCrash?.();
  restoreCrash = undefined;
}

/** What the slot holds now: the header bar and the content area, as the host sees them. */
export function slotState() {
  return {
    bar: slotHost?.children[0]?.textContent ?? '',
    barHeaders: slotHost?.children[0]?.querySelectorAll('header').length ?? 0,
    main: slotHost?.children[1]?.textContent ?? '',
    mainHeaders: slotHost?.children[1]?.querySelectorAll('header').length ?? 0,
    alert: slotHost?.querySelector('[role="alert"]')?.textContent ?? '',
  };
}

/** What is still running that the module started. */
export function leaks() {
  const started = window.__started!;
  return {
    openSockets: started.sockets.filter((socket) => socket.readyState < WebSocket.CLOSING).length,
    intervals: started.intervals.size,
    workers: started.workers.filter((worker) => !worker.terminated).length,
    channels: started.channels.filter((channel) => !channel.closed).length,
    sockets: started.sockets.length,
    // The host owns the slot and its route element; the React tree under it must be gone.
    mountedChildren: slotHost?.querySelector('.colab-route')?.childElementCount ?? 0,
  };
}
