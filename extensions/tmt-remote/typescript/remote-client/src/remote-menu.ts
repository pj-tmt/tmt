/** Remote's module menu in the TMT web shell: one row per remote, then adding a remote and the
 * shared Remote settings. Navigation and observations remain page-owned. */
import type { ShellMenuItem } from '../../../../../design/browser-ui/src/shell.js';
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
/** `shell` is the mounted route's part of the app; a page bundled apart (settings) is handed it. */
export function remoteMenu(actions: Actions, shell: RemoteShell | undefined = remoteShell()) {
  if (!shell) return () => {};
  const page = document.documentElement.dataset.tmtPage;
  const landing = new URL('/remote/', location.href).href;
  const settings = new URL('/remote/settings', location.href).href;
  let catalog: RemoteCatalog = { version: 1, entries: [] };
  let state = 'Checking';
  let address = '';
  const run = new Map<string, () => void>();
  function paint() {
    run.clear();
    const view = remoteState(state);
    const reason = actions.blocked();
    const selected = catalog.entries.find((entry) => currentRemote(entry, address, location.href));
    const entries = [...catalog.entries].sort(
      (a, b) => Number(a.kind === 'firestore') - Number(b.kind === 'firestore'),
    );
    const remotes: ShellMenuItem[] = [];
    const onLanding = page !== 'settings';
    if (!selected)
      remotes.push({
        id: 'current',
        label: [view.mark, `this machine · ${view.word}`].filter(Boolean).join(' '),
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
          { label: 'Remotes', items: remotes },
          // Managing the machine this page runs on, under its own heading (c5ccd17a: management
          // sits under that remote); each item opens its part of the shared settings page.
          {
            label: 'This machine',
            items: [
              {
                id: 'devices',
                label: 'Devices',
                href: `${settings}#devices-title`,
                current: false,
              },
              { id: 'pair', label: 'Connect a device', href: `${landing}#heading`, current: false },
              {
                id: 'machine',
                label: 'This machine’s settings',
                href: `${settings}#opening-title`,
                current: false,
              },
            ],
          },
          {
            items: [
              { id: 'add', label: 'Add remote…', href: `${settings}#add-remote`, current: false },
              { id: 'settings', label: 'Remote settings', href: settings, current: !onLanding },
            ],
          },
        ],
      },
      (id) => {
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
  // The registry and the blocked reason change outside this menu: refresh when the pointer or
  // keyboard reaches it, as the old switcher did on opening.
  // The shell renders the menu element and may replace it, so the page listens on the document
  // until it unmounts (pagehide).
  const refresh = () => {
    paint();
    void actions.refresh();
  };
  const lifetime = new AbortController();
  addEventListener('pagehide', () => lifetime.abort(), { once: true });
  const entered = (event: PointerEvent | FocusEvent) => {
    const menu = (event.target as Element | null)?.closest?.('.tmt-ui-module-menu');
    if (menu && !menu.contains(event.relatedTarget as Node | null)) refresh();
  };
  document.addEventListener('pointerover', entered, { signal: lifetime.signal });
  document.addEventListener('focusin', entered, { signal: lifetime.signal });
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
