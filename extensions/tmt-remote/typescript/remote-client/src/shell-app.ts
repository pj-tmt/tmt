/**
 * The one TMT web app the door serves at every TMT route (#3937): one document, one shell, one
 * router. Remote's own pages are route modules here; Colab is loaded lazily from its own mount.
 */
import {
  startShellHost,
  type ShellHost,
  type ShellRoute,
  type ShellRouteModule,
  type ShellSlot,
} from '../../../../../design/browser-ui/src/shell-host.js';
import type { ShellAnswer } from '../../../../../design/browser-ui/src/shell.js';
import home from '../../../rust/tmt-remote/assets/home.html?raw';
import landing from '../../../rust/tmt-remote/assets/landing.html?raw';
import settings from '../../../rust/tmt-remote/assets/settings.html?raw';
import tmtSettings from '../../../rust/tmt-remote/assets/tmt-settings.html?raw';
import ops from '../../../rust/tmt-remote/assets/ops.html?raw';
import digest from '../../../rust/tmt-remote/assets/digest.html?raw';
import { digestPage } from './digest-page.js';
import { opsPage } from './ops-page.js';
import { homePage, hostRemote, remoteShell } from './remote-shell.js';
import { tmtSettingsPage } from './tmt-settings-page.js';
import { applyStoredTheme } from '../../../../../design/browser-ui/src/theme.js';

/**
 * Put a page's markup into the slot: its header into the shell bar, its content into the main
 * area, its title and page name onto the document. The page files stay the one source of each
 * page's markup; the door no longer serves them as documents.
 */
function adopt(slot: ShellSlot, html: string): void {
  const page = new DOMParser().parseFromString(html, 'text/html');
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
}

/** A Remote page as a route module: its markup, then its page code, released on unmount. */
function remotePage(
  html: string,
  run: (slot: ShellSlot) => void | (() => void) | Promise<void>,
): ShellRouteModule {
  return {
    async mount(slot) {
      adopt(slot, html);
      hostRemote(slot);
      const release = await run(slot);
      return () => {
        release?.();
        hostRemote(undefined);
        // Page code ends its sessions and checks on pagehide, as when the document unloads.
        window.dispatchEvent(new PageTransitionEvent('pagehide', { persisted: false }));
      };
    },
  };
}

/** Colab's module entry, under its own mount, only from the composed answer (never unpaired). */
function colabEntry(answer: ShellAnswer | undefined): string | undefined {
  const colab = answer?.modules.find((module) => module.extension === 'colab');
  if (!colab) return undefined;
  const mount = new URL(colab.rail.path, location.href);
  const entry = new URL((colab as { entry?: string }).entry ?? 'assets/colab-module.js', mount);
  // Confined to the module's own mount; anything else falls back to the module link.
  return entry.origin === location.origin && entry.pathname.startsWith(mount.pathname)
    ? entry.href
    : undefined;
}

function message(slot: ShellSlot, title: string, text: string): void {
  const main = document.createElement('main');
  main.className = 'page';
  const heading = document.createElement('h2');
  heading.textContent = title;
  const body = document.createElement('p');
  body.textContent = text;
  main.append(heading, body);
  slot.main.replaceChildren(main);
  slot.setTitle(title);
}

export const routes: readonly ShellRoute[] = [
  {
    id: 'home',
    paths: ['/'],
    rail: 'home',
    load: async () => remotePage(home, () => homePage(remoteShell()!)),
  },
  {
    id: 'remote',
    paths: ['/remote', '/remote/'],
    rail: 'remote',
    // The landing reads nothing from the door until its own check connects.
    quiet: true,
    load: async () =>
      remotePage(landing, async () => {
        const { landingPage } = await import('./browser.js');
        await landingPage();
      }),
  },
  {
    id: 'remote-settings',
    paths: ['/remote/settings'],
    rail: 'remote',
    load: async () =>
      remotePage(settings, async () => {
        const { settingsPage } = await import(/* @vite-ignore */ settingsEntry);
        await settingsPage();
      }),
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
    load: async () => remotePage(digest, () => digestPage()),
  },
  {
    id: 'settings',
    paths: ['/settings', '/settings/'],
    rail: 'settings',
    load: async () => remotePage(tmtSettings, () => tmtSettingsPage()),
  },
  {
    id: 'colab',
    paths: ['/colab', '/colab/'],
    rail: 'colab',
    async load(_remembered, settled) {
      // Colab loads only from what this page's own read describes: never unpaired.
      const entry = colabEntry(await settled());
      if (!entry)
        return {
          mount: (slot) =>
            message(
              slot,
              'Colab',
              'Pair this browser with Remote to open Colab. On the machine that runs Remote, run tmt remote pair.',
            ),
        };
      try {
        return (await import(/* @vite-ignore */ entry)) as ShellRouteModule;
      } catch {
        return {
          mount: (slot) =>
            message(
              slot,
              'Colab',
              'Colab could not start. It may have stopped on your machine. Reload to try again.',
            ),
        };
      }
    },
  },
];

/** Remote's settings page code, served beside the SDK. */
const settingsEntry = '/sdk/settings-v1.js';

let started: ShellHost | undefined;
/** Start the app once in the served shell document. */
export function shellApp(): ShellHost | undefined {
  const root = document.querySelector<HTMLElement>('.tmt-ui-shell');
  if (!root || started) return started;
  applyStoredTheme();
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
