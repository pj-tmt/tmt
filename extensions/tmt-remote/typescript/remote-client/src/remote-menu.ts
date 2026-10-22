/** Remote's module menu in the TMT web shell: one row per remote, then adding a remote and the
 * shared Remote settings. Navigation and observations remain page-owned. */
import type { ShellMenuItem } from '@tmt/browser-ui/shell';
import type { RemoteCatalog, RemoteReference } from './remote-registry.js';
import { remoteState, currentRemote } from './remote-state.js';
import { remoteShell, type RemoteShell } from './remote-shell.js';

interface Actions {
  /** Re-read the registry (another tab may have changed it); the page repaints the menu. */
  refresh(): Promise<void>;
  open(entry: RemoteReference): void;
  add(): void;
  recover(action: string): void;
  blocked(): string;
}
/**
 * The menu rows that open a part of a page. Remote's pages treat any URL fragment as untrusted
 * input and erase it, so a part is never addressed by `#id`: the menu remembers the chosen part
 * for this tab and scrolls to it, on this page or once the other page mounts.
 */
const PARTS = {
  devices: { page: 'settings', target: 'devices-title' },
  machine: { page: 'settings', target: 'opening-title' },
  pair: { page: 'landing', target: 'heading' },
} as const;
type Part = keyof typeof PARTS;
const PART_KEY = 'tmt.remote.menu.part';
function chosenPart(): Part | undefined {
  try {
    const value = sessionStorage.getItem(PART_KEY);
    return value && value in PARTS ? (value as Part) : undefined;
  } catch {
    return undefined;
  }
}
function choosePart(part: Part | undefined): void {
  try {
    if (part) sessionStorage.setItem(PART_KEY, part);
    else sessionStorage.removeItem(PART_KEY);
  } catch {
    // The choice still applies to this page.
  }
}

/** `shell` is the mounted route's part of the app; a page bundled apart (settings) is handed it. */
function showPart(part: Part): void {
  const target = document.getElementById(PARTS[part].target);
  target?.scrollIntoView({ block: 'start' });
}

export function remoteMenu(actions: Actions, shell: RemoteShell | undefined = remoteShell()) {
  if (!shell) return () => {};
  const page = document.documentElement.dataset.tmtPage;
  const landing = new URL('/remote/', location.href).href;
  const settings = new URL('/remote/settings', location.href).href;
  let catalog: RemoteCatalog = { version: 1, entries: [] };
  let state = 'Checking';
  let address = '';
  const run = new Map<string, () => void>();
  const chosen = chosenPart();
  choosePart(undefined);
  let shownPart: Part | undefined = chosen && PARTS[chosen].page === page ? chosen : undefined;
  function paint() {
    run.clear();
    const view = remoteState(state);
    const reason = actions.blocked();
    const selected = catalog.entries.find((entry) => currentRemote(entry, address, location.href));
    const entries = [...catalog.entries].sort(
      (a, b) => Number(a.kind === 'firestore') - Number(b.kind === 'firestore'),
    );
    const remotes: ShellMenuItem[] = [];
    // The highlighted row is the part chosen on this page, else the page itself.
    const onSettings = page === 'settings';
    const here = shownPart;
    const lit = (id: Part) => here === id;
    const onLanding = !onSettings && !here;
    if (!selected)
      remotes.push({
        id: 'current',
        label: [view.mark, `This machine · ${view.word}`].filter(Boolean).join(' '),
        href: landing,
        current: onLanding,
      });
    for (const entry of entries) {
      const here = currentRemote(entry, address, location.href);
      const shown = remoteState(state, here);
      const id = `remote:${entry.name}`;
      if (!here) run.set(id, () => actions.open(entry));
      remotes.push({
        id,
        label: here
          ? [shown.mark, `${entry.name} · ${shown.word}`].filter(Boolean).join(' ')
          : `Open ${entry.name} · ${entry.kind === 'local' ? 'Local' : `Firebase · ${entry.project}`} · not checked`,
        href: here ? landing : '#',
        current: here && onLanding,
        disabled: !here && !!reason,
        note: !here && reason ? reason : undefined,
      });
    }
    if (view.action) {
      run.set('recover', () => actions.recover(remoteState(state).action));
      remotes.push({
        id: 'recover',
        label: view.action,
        href: '#',
        current: false,
        note: view.text,
      });
    }
    run.set('add', actions.add);
    shell!.setMenu(
      {
        title: 'Remote',
        groups: [
          { items: remotes },
          // Managing the machine this page runs on, under its own heading (c5ccd17a: management
          // sits under that remote); each item opens its part of the shared settings page.
          {
            label: 'This machine',
            items: [
              {
                id: 'devices',
                label: 'Devices',
                href: settings,
                current: lit('devices'),
              },
              {
                id: 'pair',
                label: 'Connect a device',
                href: landing,
                current: lit('pair'),
              },
              {
                id: 'machine',
                label: 'This machine’s settings',
                href: settings,
                current: lit('machine'),
              },
            ],
          },
          {
            items: [
              {
                id: 'add',
                label: 'Add remote…',
                href: `${settings}#add-remote`,
                current: false,
              },
              {
                id: 'settings',
                label: 'Remote settings',
                href: settings,
                current: onSettings && !here,
              },
            ],
          },
        ],
      },
      (id) => {
        if (id in PARTS) {
          const part = id as Part;
          // Another page: the app opens it, and that page's menu scrolls to the part.
          if (PARTS[part].page !== page) return (choosePart(part), false);
          shownPart = part;
          showPart(part);
          paint();
          return true;
        }
        if (id === 'settings' || id === 'current') {
          if ((id === 'settings') === onSettings) {
            shownPart = undefined;
            window.scrollTo({ top: 0 });
            paint();
            return true;
          }
          return false;
        }
        const action = run.get(id);
        if (!action) return false;
        // A request may have started since the last paint; a blocked row stays put.
        if (!id.startsWith('remote:') || !actions.blocked()) action();
        else paint();
        return true;
      },
    );
  }
  paint();
  // A part chosen on another page opens here once this page's markup is in place; the choice is
  // spent, so reaching this page another way (the rail, Back) opens it at the top.
  if (shownPart) {
    const opening = shownPart;
    requestAnimationFrame(() => showPart(opening));
  }
  // The registry and the blocked reason change outside this menu: refresh when the pointer or
  // keyboard reaches it, as the old switcher did on opening.
  // The shell renders the menu element and may replace it, so the page listens on the document
  // until its route unmounts.
  const refresh = () => {
    paint();
    void actions.refresh();
  };
  const entered = (event: PointerEvent | FocusEvent) => {
    const menu = (event.target as Element | null)?.closest?.('.tmt-ui-module-menu');
    if (menu && !menu.contains(event.relatedTarget as Node | null)) refresh();
  };
  document.addEventListener('pointerover', entered, { signal: shell.signal });
  document.addEventListener('focusin', entered, { signal: shell.signal });
  // The rail fills from the owner's composed shell only once this page may send work: settings
  // always talks to the door; the native entry waits until its own check connects, so an
  // unpaired entry sends nothing.
  if (page === 'settings') shell.describe();
  return (next: RemoteCatalog, observed: string, checkedAddress: string) => {
    catalog = next;
    state = observed;
    address = checkedAddress;
    if (observed === 'Connected' && checkedAddress) shell.describe(checkedAddress);
    paint();
  };
}
