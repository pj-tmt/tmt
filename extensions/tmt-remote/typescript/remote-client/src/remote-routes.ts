/** Public Remote views. Router, tabs and module menu consume this one declaration. */
import type { ViewerRole } from './remote-state.js';

export type RemoteView =
  | 'remotes'
  | 'create'
  | 'join'
  | 'overview'
  | 'members'
  | 'my-devices'
  | 'settings'
  | 'service';
export interface RemoteItem {
  readonly id: RemoteView;
  readonly path: string;
  readonly label: string;
  readonly title: string;
  readonly page: 'landing' | 'settings';
  readonly adminOnly?: boolean;
}
export const REMOTE_ITEMS: readonly RemoteItem[] = [
  { id: 'remotes', path: '/remote/', label: 'My Remotes', title: 'My Remotes', page: 'settings' },
  {
    id: 'create',
    path: '/remote/create',
    label: 'Create a Remote',
    title: 'Create a Remote',
    page: 'settings',
  },
  {
    id: 'join',
    path: '/remote/join',
    label: 'Join a Remote',
    title: 'Join a Remote',
    page: 'settings',
  },
  {
    id: 'overview',
    path: '/remote/overview',
    label: 'Overview',
    title: 'Overview',
    page: 'landing',
  },
  { id: 'members', path: '/remote/members', label: 'People', title: 'People', page: 'settings' },
  {
    id: 'my-devices',
    path: '/remote/my-devices',
    label: 'My devices',
    title: 'My devices',
    page: 'settings',
  },
  {
    id: 'service',
    path: '/remote/service',
    label: 'Service & upgrade',
    title: 'Service & upgrade',
    page: 'settings',
    adminOnly: true,
  },
  {
    id: 'settings',
    path: '/remote/settings',
    label: 'Settings',
    title: 'Settings',
    page: 'settings',
  },
];
export const REMOTE_PATHS: readonly string[] = [
  '/remote',
  ...REMOTE_ITEMS.filter((item) => item.id !== 'remotes').map((item) => item.path),
  // The shared shell uses the last registered path as this module's rail/default entry.
  remotePath('remotes'),
];
export function remotePath(id: RemoteView): string {
  return REMOTE_ITEMS.find((item) => item.id === id)!.path;
}
/** Removed pre-launch paths have no alias or redirect. */
export function remoteItem(pathname: string): RemoteItem | undefined {
  return REMOTE_ITEMS.find(
    (item) => item.path === pathname || (pathname === '/remote' && item.id === 'remotes'),
  );
}
/** Display-only locking: signed management remains the authority for every effect. */
export function remoteTabAccess(
  item: RemoteItem,
  role: ViewerRole | undefined,
): { locked: boolean; reason: string; request: string } {
  if (!item.adminOnly || role === 'owner') return { locked: false, reason: '', request: '' };
  if (role !== 'user')
    return {
      locked: true,
      reason: 'Access is not confirmed. Check again before making changes.',
      request: '',
    };
  return {
    locked: true,
    reason:
      "Only an admin can manage this Remote's service and upgrade steps. Ask the person who manages this Remote.",
    request: "Please review this Remote's service and upgrade options with me.",
  };
}
