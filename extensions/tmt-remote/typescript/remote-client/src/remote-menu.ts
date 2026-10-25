/** Remote's module menu links to the one My Remotes list and this machine's views.
 * Every row is a route of its own
 * (`remote-routes.ts`) and the selected row is a function of the path: the menu keeps no state of
 * its own about where the page is. Observations remain page-owned. */
import type { ShellMenuItem } from '@tmt/browser-ui/shell';
import type { RemoteRegistryView } from './remote-registry.js';
import { remoteShell, type RemoteShell } from './remote-shell.js';
import { REMOTE_ITEMS, remoteItem, type RemoteView } from './remote-routes.js';

interface Actions {
  /** Re-read the registry (another tab may have changed it); the page repaints the menu. */
  refresh(): Promise<void>;
  recover(action: string): void;
}
export function remoteMenu(actions: Actions, shell: RemoteShell | undefined = remoteShell()) {
  if (!shell) return () => {};
  const href = (view: RemoteView) =>
    new URL(REMOTE_ITEMS.find((item) => item.id === view)!.path, location.href).href;
  let projection: RemoteRegistryView | undefined;
  const run = new Map<string, () => void>();
  function paint() {
    run.clear();
    const view = projection?.current.connection;
    const showing = remoteItem(location.pathname)?.id;
    const item = (id: RemoteView): ShellMenuItem => {
      const row = REMOTE_ITEMS.find((candidate) => candidate.id === id)!;
      return { id, label: row.label, href: href(id), current: showing === id };
    };
    const remotes: ShellMenuItem[] = [
      {
        ...item('overview'),
        label: `This Remote · ${view?.text ?? 'Checking'}`,
        current: showing !== 'remotes',
      },
    ];
    if (view?.action) {
      run.set('recover', () => actions.recover(view.action));
      remotes.push({
        id: 'recover',
        label: view.action,
        href: '#',
        current: false,
        note: view.text,
      });
    }
    shell!.setMenu(
      {
        title: 'Remote',
        groups: [
          { items: [item('remotes')] },
          // Managing the machine this page runs on, under its own heading (c5ccd17a: management
          // sits under that remote); each row is a route of its own.
          {
            label: 'This machine',
            items: remotes,
          },
        ],
      },
      (id) => {
        // A row for a route returns false: the shell router navigates and the page follows the path.
        const action = run.get(id);
        if (!action) return false;
        action();
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
  return (next: RemoteRegistryView) => {
    projection = next;
    if (next.current.connection.word === 'Connected' && next.address) shell.describe(next.address);
    paint();
  };
}
