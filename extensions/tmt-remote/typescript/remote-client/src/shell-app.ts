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

/**
 * Put a page's markup into the slot: its header into the shell bar, its content into the main
 * area, its title and page name onto the document. The page files stay the one source of each
 * page's markup; the door no longer serves them as documents.
 */
async function adopt(slot: ShellSlot, html: string): Promise<() => void> {
  const page = new DOMParser().parseFromString(html, 'text/html');
  // A page's own stylesheet loads before its markup shows, and goes with the route.
  const release = await pageStyles(page);
  const bar = page.querySelector('.tmt-ui-shell-bar');
  const main = page.querySelector('.tmt-ui-shell-main');
  slot.bar.replaceChildren(
    ...(bar ? [...bar.childNodes].map((node) => document.adoptNode(node)) : []),
  );
  slot.main.replaceChildren(
    ...(main ? [...main.childNodes].map((node) => document.adoptNode(node)) : []),
  );
  document.documentElement.dataset.tmtPage = page.documentElement.dataset.tmtPage ?? '';
  slot.setTitle(page.title.replace(/ · TMT$/, ''));
  return release;
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
 * slot (each page's `main` marked `data-page`), the connection page's header in the bar. The
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
  let settingsSheets: Promise<() => void> | undefined;
  settingsStyles.load = async () => {
    settingsSheets ??= pageStyles(settingsPage);
    await settingsSheets;
  };
  const bar = landingPage.querySelector('.tmt-ui-shell-bar');
  slot.bar.replaceChildren(
    ...(bar ? [...bar.childNodes].map((node) => document.adoptNode(node)) : []),
  );
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
      const lifetime = new AbortController();
      hostRemote(slot, lifetime.signal);
      const end = () => {
        // The page's own code ends on its route's signal; nothing else in the document hears it.
        lifetime.abort();
        unstyle();
        hostRemote(undefined);
      };
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
              landing: async (lifetime, registry) => {
                const { landingPage } = await import('./browser.js');
                await landingPage(lifetime, registry);
              },
              settings: async (shell, registry) => {
                const { settingsPage } = await import(/* @vite-ignore */ settingsEntry);
                await settingsPage(shell, registry);
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
  started = startShellHost(root, routes, {
    failed: (main) =>
      main.replaceChildren(
        Object.assign(document.createElement('p'), {
          className: 'page',
          textContent: 'This page could not load. Reload to try again.',
        }),
      ),
  });
  return started;
}
