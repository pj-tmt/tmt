/** Strict browser projection of the existing Remote readiness table; no provider I/O. */
export type FirestoreState = 'enabled' | 'not-enabled' | 'unknown';
export interface FirestorePrerequisite {
  item: string;
  state: FirestoreState;
  reason?: string;
  next?: 'tmt remote deploy firestore';
}
export interface FirestoreLayerView {
  layer: 'sharing' | 'operations' | 'attachments';
  state: FirestoreState;
  prerequisites: FirestorePrerequisite[];
}
export const firestoreTitles = {
  sharing: 'Page sharing',
  operations: 'Operation sync',
  attachments: 'Attachment storage',
};
export const firestoreItems: Record<string, string> = {
  project: 'Project',
  'sign-in': 'Sign-in',
  rules: 'Rules',
  'plan-tier': 'Firebase plan',
  quota: 'Free-plan quota',
  support: 'Availability in this release',
};
const deploy = 'tmt remote deploy firestore';
// Reason vocabulary follows readiness::ROWS; copy is fixed, never provider text.
export const firestoreReasons: Record<string, Record<string, [string, boolean]>> = {
  project: {
    'not-configured': ['No Firebase project is set up.', true],
    'access-lost': ['tmt can no longer reach the Firebase project.', true],
    'not-checked': ['The Firebase project has not been checked.', false],
  },
  'sign-in': {
    'provider-disabled': ['Sign-in is not enabled for the project.', true],
    'permission-missing': ["This account isn't allowed to turn on sign-in for the project.", false],
    'not-checked': ['Sign-in has not been checked.', false],
  },
  rules: {
    'not-deployed': ['Firestore rules are not deployed.', true],
    'out-of-date': ['Firestore rules are out of date.', true],
    partial: ['Firestore rules are only partly deployed.', true],
    'not-checked': ['Firestore rules have not been checked.', false],
  },
  'plan-tier': {
    'paid-plan-required': ['This needs a paid Firebase plan.', false],
    'not-checked': ['The Firebase plan has not been checked.', false],
  },
  quota: {
    'headroom-low': ["Today's free Firebase quota is almost used up.", false],
    exhausted: ["Today's free Firebase quota is used up.", false],
    'not-checked': ['The free Firebase quota has not been checked.', false],
  },
  support: { 'not-implemented': ["This isn't available in this release yet.", false] },
};
const specs = [
  ['sharing', ['project', 'sign-in', 'rules', 'plan-tier', 'quota']],
  ['operations', ['plan-tier', 'support']],
  ['attachments', ['support', 'project', 'rules', 'quota']],
] as const;
function requireValid(condition: unknown): asserts condition {
  if (!condition) throw new Error('Invalid Firestore readiness response.');
}
function object(value: unknown): Record<string, unknown> {
  requireValid(typeof value === 'object' && value !== null && !Array.isArray(value));
  return value as Record<string, unknown>;
}
function keys(row: Record<string, unknown>, expected: string[]): void {
  requireValid(
    Object.keys(row).length === expected.length && expected.every((key) => Object.hasOwn(row, key)),
  );
}
export function parseFirestoreLayers(value: unknown): FirestoreLayerView[] {
  requireValid(Array.isArray(value));
  if (!value.length) return [];
  requireValid(value.length === specs.length);
  for (const [index, [layer, items]] of specs.entries()) {
    const row = object(value[index]);
    keys(row, ['layer', 'state', 'prerequisites']);
    requireValid(
      row.layer === layer &&
        Array.isArray(row.prerequisites) &&
        row.prerequisites.length === items.length,
    );
    const states: string[] = [];
    for (const [i, item] of items.entries()) {
      const entry = object(row.prerequisites[i]);
      requireValid(
        entry.item === item &&
          ['enabled', 'not-enabled', 'unknown'].includes(entry.state as string),
      );
      states.push(entry.state as string);
      if (entry.state === 'enabled') {
        keys(entry, ['item', 'state']);
        continue;
      }
      const reason =
        typeof entry.reason === 'string' && Object.hasOwn(firestoreReasons[item]!, entry.reason)
          ? firestoreReasons[item]![entry.reason]
          : undefined;
      requireValid(reason && (entry.state === 'unknown') === (entry.reason === 'not-checked'));
      requireValid(entry.reason !== 'paid-plan-required' || layer === 'operations');
      keys(entry, reason[1] ? ['item', 'state', 'reason', 'next'] : ['item', 'state', 'reason']);
      requireValid(entry.next === (reason[1] ? deploy : undefined));
    }
    const aggregate = states.includes('not-enabled')
      ? 'not-enabled'
      : states.includes('unknown')
        ? 'unknown'
        : 'enabled';
    requireValid(row.state === aggregate);
  }
  return value as FirestoreLayerView[];
}
