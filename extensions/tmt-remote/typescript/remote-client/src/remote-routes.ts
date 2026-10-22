/**
 * Remote's sub-routes: the one table both the router (`shell-app.ts`) and the module menu read.
 * Each row is its own path under the one `remote` route, so a switch between them only swaps what
 * the mounted page shows; the menu's selected row is whichever row the path names.
 */
export type RemoteView = 'status' | 'connect' | 'devices' | 'machine' | 'settings';

export interface RemoteItem {
  readonly id: RemoteView;
  readonly path: string;
  /** The menu row. */
  readonly label: string;
  /** The page heading (`h1`) and document title. */
  readonly title: string;
  /** Which page's markup holds the view: the connection check, or the settings forms. */
  readonly page: 'landing' | 'settings';
}

export const REMOTE_ITEMS: readonly RemoteItem[] = [
  { id: 'status', path: '/remote/', label: 'Status', title: 'Remote connection', page: 'landing' },
  {
    id: 'devices',
    path: '/remote/devices',
    label: 'Devices',
    title: 'Devices',
    page: 'settings',
  },
  {
    id: 'connect',
    path: '/remote/connect',
    label: 'Connect a device',
    title: 'Connect a device',
    page: 'landing',
  },
  {
    id: 'machine',
    path: '/remote/machine',
    label: 'This machine’s settings',
    title: 'This machine’s settings',
    page: 'settings',
  },
  {
    id: 'settings',
    path: '/remote/settings',
    label: 'Remote settings',
    title: 'Remote settings',
    page: 'settings',
  },
];

/**
 * The route's paths. `/remote/` comes last: the rail links to a route's last path, and `/remote`
 * (no slash) is the same row.
 */
export const REMOTE_PATHS: readonly string[] = [
  ...REMOTE_ITEMS.filter((item) => item.id !== 'status').map((item) => item.path),
  '/remote',
  '/remote/',
];

/** The row a path names; `/remote` and `/remote/` are Status, anything else under it is none. */
export function remoteItem(pathname: string): RemoteItem | undefined {
  const path = pathname === '/remote' ? '/remote/' : pathname.replace(/(?<=.)\/$/, '');
  return REMOTE_ITEMS.find((item) => item.path === path || item.path === `${path}/`);
}
