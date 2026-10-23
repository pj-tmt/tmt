/** Remote's module menu in the TMT web shell: one row per remote, the rows for this machine, then
 * adding a remote and the shared Remote settings. Every row is a route of its own
 * (`remote-routes.ts`) and the selected row is a function of the path: the menu keeps no state of
 * its own about where the page is. Observations remain page-owned. */
import type { ShellMenuItem } from '@tmt/browser-ui/shell';
import type { RemoteCatalog, RemoteReference } from './remote-registry.js';
import { remoteState, currentRemote } from './remote-state.js';
import { remoteShell, type RemoteShell } from './remote-shell.js';
import { REMOTE_ITEMS, remoteItem, type RemoteView } from './remote-routes.js';

interface Actions {
  /** Re-read the registry (another tab may have changed it); the page repaints the menu. */
  refresh(): Promise<void>;
  open(entry: RemoteReference): void;
  add(): void;
  recover(action: string): void;
  blocked(): string;
}
export function remoteMenu(actions: Actions, shell: RemoteShell | undefined = remoteShell()) {
  if (!shell) return () => {};
  const href = (view: RemoteView) =>
    new URL(REMOTE_ITEMS.find((item) => item.id === view)!.path, location.href).href;
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
    const showing = remoteItem(location.pathname)?.id;
    const item = (id: RemoteView): ShellMenuItem => {
      const row = REMOTE_ITEMS.find((candidate) => candidate.id === id)!;
      return { id, label: row.label, href: href(id), current: showing === id };
    };
    const remotes: ShellMenuItem[] = [];
    if (!selected)
      remotes.push({
        id: 'status',
        label: [view.mark, `Status · ${view.word}`].filter(Boolean).join(' '),
        href: href('status'),
        current: showing === 'status',
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
        href: here ? href('status') : '#',
        current: here && showing === 'status',
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
          // sits under that remote); each row is a route of its own.
          {
            label: 'This machine',
            items: [item('devices'), item('connect'), item('machine')],
          },
          {
            items: [
              {
                id: 'add',
                label: 'Add remote…',
                href: href('settings'),
                current: false,
              },
              item('settings'),
            ],
          },
        ],
      },
      (id) => {
        // A row for a route returns false: the shell router navigates and the page follows the path.
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
  // Back, forward, the rail and a click on a row all end in the path changing under the same
  // mounted route; the shell announces it, and the menu follows it.
  window.addEventListener('popstate', paint, { signal: shell.signal });
  // The registry and the blocked reason change outside this menu: refresh when the pointer or
  // keyboard reaches it, as the old switcher did on opening.
  // The shell renders the menu element and may replace it, so the page listens on the document
  // until its route unmounts.
  const refresh = () => {
    paint();
    void actions.refresh();
  };
  const entered = (event: PointerEvent | FocusEvent) => {
    const menu = (event.target as Element | null)?.closest?.(
      '.tmt-ui-module-menu, .tmt-ui-rail-sub',
    );
    if (menu && !menu.contains(event.relatedTarget as Node | null)) refresh();
  };
  document.addEventListener('pointerover', entered, { signal: shell.signal });
  document.addEventListener('focusin', entered, { signal: shell.signal });
  return (next: RemoteCatalog, observed: string, checkedAddress: string) => {
    catalog = next;
    state = observed;
    address = checkedAddress;
    if (observed === 'Connected' && checkedAddress) shell.describe(checkedAddress);
    paint();
  };
}
