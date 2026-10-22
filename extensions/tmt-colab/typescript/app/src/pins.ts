import { ManagementError } from './management.js';
import { text } from './strings.js';

const t = text.pageList;

/** The owner-list state projected by PageProjects; absent until this page can be pinned. */
export interface PagePinState {
  pinned: boolean;
  busy: boolean;
  failure?: string;
}

export interface PagePinRequest {
  id: number;
  action: 'toggle' | 'dismiss';
}

/** Phones expose the hidden header pin in More; the menu supplies no write authority. */
export function pinMenuItem(
  state: PagePinState | undefined,
  compact: boolean,
  canWrite: boolean,
  readOnly: boolean,
): { key: 'pin'; label: string; disabled: boolean } | undefined {
  if (!compact || !canWrite || readOnly || !state) return;
  return {
    key: 'pin' as const,
    label: state.pinned ? t.unpinItem : t.pinItem,
    disabled: state.busy,
  };
}

/** The words for a refused or failed pin, whichever surface asked for it. */
export function pinFailureText(failure: unknown): string {
  const code = failure instanceof ManagementError ? failure.code : 'UNKNOWN';
  if (code === 'CAPACITY') return t.pinFull;
  return t.pinFailures[code as keyof typeof t.pinFailures] ?? t.pinFailures.UNKNOWN;
}
