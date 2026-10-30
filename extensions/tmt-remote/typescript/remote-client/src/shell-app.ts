/**
 * The one TMT web app the door serves at every TMT route (#3937): one document, one shell, one
 * router. Remote's own pages are route modules here; Colab is loaded lazily from its own mount.
 */
import {
  appHref,
  appRail,
  startShellHost,
  type ShellHost,
  type ShellRoute,
  type ShellRouteModule,
  type ShellSlot,
} from '@tmt/browser-ui/shell-host';
import { mountShell, rememberedShellAnswer, type ShellAnswer } from '@tmt/browser-ui/shell';
import home from '../../../rust/tmt-remote/assets/home.html?raw';
import landing from '../../../rust/tmt-remote/assets/landing.html?raw';
import settings from '../../../rust/tmt-remote/assets/settings.html?raw';
import tmtSettings from '../../../rust/tmt-remote/assets/tmt-settings.html?raw';
import ops from '../../../rust/tmt-remote/assets/ops.html?raw';
import digest from '../../../rust/tmt-remote/assets/digest.html?raw';
import { digestPage } from './digest-page.js';
import { moduleLoad, type ModuleLoad } from './module-entry.js';
import { opsPage } from './ops-page.js';
import { remoteApp } from './remote-app.js';
import { REMOTE_PATHS } from './remote-routes.js';
import { homePage, hostRemote, remoteShell } from './remote-shell.js';
import { tmtSettingsPage } from './tmt-settings-page.js';
import { applyStoredTheme } from '@tmt/browser-ui/theme';
import { navigationSnapshot, RootSearch, type SearchModule } from './root-search.js';

/** Root-private cancellation; modules still consume the existing shared slot contract. */
interface RootSlot extends ShellSlot {
  readonly routeSignal: AbortSignal;
}

/**
 * Put a page's markup into the slot: actions into the common bar, heading and content into the
 * main area, its title and page name onto the document. The page files stay the one source of each
 * page's markup; the door no longer serves them as documents.
 */
async function adopt(slot: ShellSlot, html: string): Promise<() => void> {
  const page = new DOMParser().parseFromString(html, 'text/html');
  // A page's own stylesheet loads before its markup shows, and goes with the route.
  const release = await pageStyles(page);
  if (!slot.topBarReady()) {
    release();
    return () => {};
  }
  const bar = page.querySelector('.tmt-ui-shell-bar');
  const main = page.querySelector('.tmt-ui-shell-main');
  slot.main.replaceChildren(
    ...(main ? [...main.childNodes].map((node) => document.adoptNode(node)) : []),
  );
  adoptBar(slot, bar);
  document.documentElement.dataset.tmtPage = page.documentElement.dataset.tmtPage ?? '';
  slot.setTitle(page.title.replace(/ · TMT$/, ''));
  return release;
}

/** The common bar draws controls. Pages without their own hero retain a content title row. */
function adoptBar(slot: ShellSlot, source: Element | null): void {
  const { actions } = slot.setTopBar();
  const header = source?.querySelector('header');
  if (!header) return;
  const row = document.createElement('div');
  row.className = 'root-title-row';
  for (const node of Array.from(header.children)) {
    if (node.matches('.header-actions,.tmt-ui-header-actions'))
      actions.append(document.adoptNode(node));
    else row.append(document.adoptNode(node));
  }
  if (row.childNodes.length) slot.main.prepend(row);
}

/**
 * The door's own stylesheets a page names beyond the shell's `/sdk/pages.css` (Remote settings has
 * `/sdk/settings.css`): linked once the route mounts, removed when it unmounts. Resolves when each
 * has loaded or failed, so the markup never shows unstyled.
 */
async function pageStyles(page: Document): Promise<() => void> {
  const added: HTMLLinkElement[] = [];
  const loading: Promise<void>[] = [];
  for (const source of page.querySelectorAll<HTMLLinkElement>('link[rel="stylesheet"]')) {
    const href = source.getAttribute('href');
    if (
      !href?.startsWith('/sdk/') ||
      href === '/sdk/pages.css' ||
      [...document.head.querySelectorAll('link')].some((link) => link.getAttribute('href') === href)
    )
      continue;
    const link = document.createElement('link');
    link.rel = 'stylesheet';
    link.href = href;
    loading.push(
      new Promise((resolve) => {
        link.onload = link.onerror = () => resolve();
      }),
    );
    document.head.append(link);
    added.push(link);
  }
  await Promise.all(loading);
  return () => added.forEach((link) => link.remove());
}

/**
 * Remote's connection check and settings forms as one document of views: both pages' markup in the
 * slot (each page's `main` marked `data-page`), the connection page's title in content. The
 * page's own code decides which view shows (`remote-app.ts`). The settings stylesheet loads with
 * the first settings view, not with the route: the connection check stays a page that asks the
 * door for nothing. `settingsStyles` resolves once it has loaded.
 */
async function adoptRemote(
  slot: ShellSlot,
  status: string,
  forms: string,
  settingsStyles: { load?: () => Promise<void> },
): Promise<() => void> {
  const parser = new DOMParser();
  const landingPage = parser.parseFromString(status, 'text/html');
  const settingsPage = parser.parseFromString(forms, 'text/html');
  const releaseLanding = await pageStyles(landingPage);
  if (!slot.topBarReady()) {
    releaseLanding();
    return () => {};
  }
  let settingsSheets: Promise<() => void> | undefined;
  settingsStyles.load = async () => {
    settingsSheets ??= pageStyles(settingsPage);
    await settingsSheets;
  };
  const bar = landingPage.querySelector('.tmt-ui-shell-bar');
  slot.main.replaceChildren(
    ...(
      [
        ['landing', landingPage],
        ['settings', settingsPage],
      ] as const
    ).flatMap(([name, page]) =>
      [...(page.querySelector('.tmt-ui-shell-main')?.childNodes ?? [])].map((node) => {
        if (node instanceof Element && node.localName === 'main')
          (node as HTMLElement).dataset.page = name;
        return document.adoptNode(node);
      }),
    ),
  );
  adoptBar(slot, bar);
  document.documentElement.dataset.tmtPage = 'landing';
  return () => {
    releaseLanding();
    void settingsSheets?.then((release) => release());
  };
}

/** A Remote page as a route module: its markup, then its page code, released on unmount. */
function remotePage(
  html: string | ((slot: ShellSlot) => Promise<() => void>),
  run: (slot: ShellSlot) => void | (() => void) | Promise<void>,
): ShellRouteModule {
  return {
    async mount(slot) {
      const unstyle = await (typeof html === 'string' ? adopt(slot, html) : html(slot));
      const routeSignal = (slot as Partial<RootSlot>).routeSignal;
      if (routeSignal?.aborted || !slot.topBarReady()) {
        unstyle();
        return;
      }
      const lifetime = new AbortController();
      hostRemote(slot, lifetime.signal, observedSession, (reason) => (heldBy = reason));
      let ended = false;
      const end = () => {
        if (ended) return;
        ended = true;
        routeSignal?.removeEventListener('abort', end);
        // The page's own code ends on its route's signal; nothing else in the document hears it.
        lifetime.abort();
        unstyle();
        hostRemote(undefined);
        heldBy = undefined;
      };
      routeSignal?.addEventListener('abort', end, { once: true });
      let release: void | (() => void);
      try {
        release = await run(slot);
      } catch (error) {
        // A page that fails to start still gives back its styles, listeners and slot.
        end();
        throw error;
      }
      return () => {
        release?.();
        end();
      };
    },
  };
}

/** The script an older Colab, which names no `entry`, is loaded from under its own mount. */
const COLAB_ENTRY = 'assets/colab-module.js';

/** Colab as the composed answer describes it: its own link, and its entry only if confined. */
function colabLoad(answer: ShellAnswer | undefined): ModuleLoad | undefined {
  const colab = answer?.modules.find((module) => module.extension === 'colab');
  if (!colab) return undefined;
  return moduleLoad(
    colab.rail.path,
    (colab as { entry?: string }).entry,
    location.href,
    COLAB_ENTRY,
  );
}

function message(
  slot: ShellSlot,
  title: string,
  text: string,
  link?: { href: string; label: string },
): void {
  // The shell's main area is the one main landmark; a message is a section inside it.
  const main = document.createElement('section');
  main.className = 'tmt-page';
  const heading = document.createElement('h2');
  heading.textContent = title;
  const body = document.createElement('p');
  body.textContent = text;
  main.append(heading, body);
  if (link) {
    const open = document.createElement('p');
    const anchor = document.createElement('a');
    anchor.href = link.href;
    anchor.textContent = link.label;
    open.append(anchor);
    main.append(open);
  }
  slot.main.replaceChildren(main);
  slot.setTitle(title);
}

export const routes: readonly ShellRoute[] = [
  {
    id: 'home',
    paths: ['/'],
    rail: 'home',
    load: async () =>
      remotePage(home, () => homePage(remoteShell()!, (href) => appHref(routes, href))),
  },
  {
    id: 'remote',
    paths: REMOTE_PATHS,
    rail: 'remote',
    // The connection check reads nothing from the door until its own check connects; the settings
    // views ask for the composed shell themselves when they first show.
    quiet: true,
    load: async () => {
      const sheets: { load?: () => Promise<void> } = {};
      return remotePage(
        (slot) => adoptRemote(slot, landing, settings, sheets),
        (slot) =>
          remoteApp(
            remoteShell()!,
            slot.main,
            slot.bar,
            () => sheets.load!(),
            async () => ({
              landing: async (lifetime, registry, connected) => {
                const { landingPage } = await import('./browser.js');
                await landingPage(lifetime, registry, connected);
              },
              settings: async (shell, registry, initial) => {
                const { settingsPage } = await import(/* @vite-ignore */ settingsEntry);
                return settingsPage(shell, registry, initial);
              },
            }),
          ),
      );
    },
  },
  {
    id: 'ops',
    paths: ['/ops', '/ops/'],
    rail: 'ops',
    load: async () => remotePage(ops, () => opsPage(remoteShell()!)),
  },
  {
    id: 'digest',
    paths: ['/digest', '/digest/'],
    rail: 'digest',
    load: async () => remotePage(digest, () => digestPage(undefined, remoteShell()!.signal)),
  },
  {
    id: 'settings',
    paths: ['/settings', '/settings/'],
    rail: 'settings',
    load: async () => remotePage(tmtSettings, () => tmtSettingsPage(remoteShell()!)),
  },
  {
    id: 'colab',
    paths: ['/colab', '/colab/'],
    rail: 'colab',
    async load(_remembered, settled) {
      // Colab loads only from what this page's own read describes: never unpaired.
      let answer = await settled();
      // A paired browser whose door restarted has no session yet: reopen it (an unpaired browser
      // fails on its local record, before any request) and read the shell again.
      if (!answer && (await reopened())) answer = await started?.redescribe();
      const load = colabLoad(answer);
      const entry = load?.entry;
      if (!entry)
        return {
          mount: (slot) =>
            load
              ? // Described, but not loadable from here: the module's own link is the way in.
                message(
                  slot,
                  'Colab',
                  'Colab could not be opened here.',
                  load.link ? { href: load.link, label: 'Open Colab' } : undefined,
                )
              : answer
                ? message(
                    slot,
                    'Colab',
                    'Colab is not running on your machine. Run tmt colab serve there, then reload.',
                  )
                : message(
                    slot,
                    'Colab',
                    'Pair this browser with Remote to open Colab. On the machine that runs Remote, run tmt remote pair.',
                    { href: '/remote/', label: 'Pair this browser' },
                  ),
        };
      try {
        return (await import(/* @vite-ignore */ entry)) as ShellRouteModule;
      } catch {
        // An older Colab has no module to load here; its own mounted app is still the way in.
        return {
          mount: (slot) =>
            message(
              slot,
              'Colab',
              'Colab could not start here. It may have stopped on your machine. Reload to try again, or open Colab on its own page.',
              load.link ? { href: load.link, label: 'Open Colab' } : undefined,
            ),
        };
      }
    },
  },
];

/** Whether this browser's pairing reopened a door session. */
async function reopened(): Promise<boolean> {
  try {
    const { reopenSession } = await import('./browser.js');
    await reopenSession();
    return true;
  } catch {
    return false;
  }
}

/** Remote's settings page code, served beside the SDK. */
const settingsEntry = '/sdk/settings-v1.js';

let started: ShellHost | undefined;
let search: RootSearch | undefined;
let session: object | undefined;
let answer: ShellAnswer | undefined;
let observed = false;
let publishSearch: (() => void) | undefined;
let heldBy: (() => string) | undefined;

/** A verified replacement retires the old search transport/providers; it grants no access. */
function observedSession(next: object): void {
  if (session === next) return;
  session = next;
  search?.replace();
  publishSearch?.();
}

/** Start the app once in the served shell document. */
export function shellApp(): ShellHost | undefined {
  const root = document.querySelector<HTMLElement>('.tmt-ui-shell');
  if (!root || started || root.dataset.tmtRail) return started;
  applyStoredTheme();
  if (document.documentElement.dataset.tmtPage === 'pair') {
    // Pairing stays its own document (its bootstrap erases the code before any other script), so
    // it shows the rail only; its links load the app's routes.
    root.dataset.tmtRail = 'static';
    const place = { origin: location.origin, href: location.href.split(/[?#]/)[0]! };
    const remote = routes.find((route) => route.id === 'remote');
    mountShell(root, { rail: appRail(rememberedShellAnswer(), routes, place, remote) });
    return undefined;
  }
  const status = document.createElement('p');
  status.className = 'root-search-status';
  status.setAttribute('role', 'status');
  status.hidden = true;
  root.append(status);
  search = new RootSearch({
    load: (entry) => import(/* @vite-ignore */ entry) as Promise<SearchModule>,
    container: () => document.body.appendChild(document.createElement('div')),
    failed: (message) => {
      status.textContent = message;
      status.hidden = false;
    },
    navigate: (href) => {
      const destination = new URL(href, location.href);
      const reason = heldBy?.();
      if (reason && started?.route(destination.pathname) !== started?.route(location.pathname)) {
        status.textContent = reason;
        status.hidden = false;
        return;
      }
      started?.navigate(href);
    },
  });
  let currentRoute: AbortController | undefined;
  const ownedRoutes = routes.map((route): ShellRoute => ({
    ...route,
    async load(remembered, settled) {
      currentRoute?.abort();
      const lifetime = new AbortController();
      currentRoute = lifetime;
      search!.replace();
      status.hidden = true;
      // The first read is the live description, never a remembered answer used for the rail.
      void settled().then((read) => {
        if (!observed) {
          observed = true;
          answer = read;
        }
        publishSearch?.();
      });
      const module = await route.load(remembered, settled);
      return {
        async mount(slot) {
          if (lifetime.signal.aborted) return;
          let active = true;
          let adopted = false;
          const publish = () => {
            if (!active || !adopted || !slot.topBarReady()) return;
            const place = { origin: location.origin, href: location.href };
            const rail = appRail(answer, routes, place);
            search!.update(
              colabLoad(answer)?.entry,
              navigationSnapshot(answer, [...rail.main, ...rail.bottom], location.href, (path) =>
                appHref(routes, path),
              ),
            );
          };
          publishSearch = publish;
          const hosted: RootSlot = {
            ...slot,
            routeSignal: lifetime.signal,
            setTopBar() {
              const bar = slot.setTopBar();
              adopted = true;
              publish();
              return bar;
            },
          };
          const release = await module.mount(hosted);
          return () => {
            active = false;
            if (publishSearch === publish) publishSearch = undefined;
            release?.();
          };
        },
      };
    },
  }));
  const host = startShellHost(root, ownedRoutes, {
    failed: (main) =>
      main.replaceChildren(
        Object.assign(document.createElement('p'), {
          className: 'page',
          textContent: 'This page could not load. Reload to try again.',
        }),
      ),
  });
  host.setCommonControls({
    openSearch: (opener) => {
      status.hidden = true;
      void search?.open(opener);
    },
    remote: { kind: 'unconfirmed', href: '/remote/' },
  });
  const dispose = () => {
    currentRoute?.abort();
    search?.dispose();
    host.dispose();
    status.remove();
    window.removeEventListener('pagehide', hide);
    window.removeEventListener('pageshow', restore);
    window.removeEventListener('popstate', changed);
    window.removeEventListener('hashchange', changed);
  };
  const changed = () => {
    search?.replace();
    publishSearch?.();
  };
  const restore = (event: PageTransitionEvent) => {
    if (event.persisted) location.reload();
  };
  const hide = () => {
    dispose();
    // A BFCache document has ended sessions and listeners: reopen through its normal entry.
    window.addEventListener('pageshow', restore, { once: true });
  };
  started = {
    ...host,
    async redescribe() {
      search?.replace();
      const read = await host.redescribe();
      answer = read;
      observed = true;
      publishSearch?.();
      return read;
    },
    dispose,
  };
  window.addEventListener('pagehide', hide, { once: true });
  window.addEventListener('popstate', changed);
  window.addEventListener('hashchange', changed);
  return started;
}
