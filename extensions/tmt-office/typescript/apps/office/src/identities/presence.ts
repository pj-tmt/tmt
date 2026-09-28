/** Endpoint observations use the native binding domain's states, not agent activity. */
export type Presence = 'active' | 'offline' | 'unknown';

export function isPresence(value: unknown): value is Presence {
  return value === 'active' || value === 'offline' || value === 'unknown';
}

export const presenceLabel: Record<Presence, string> = {
  active: 'Online',
  offline: 'Offline',
  unknown: 'Presence unknown',
};
