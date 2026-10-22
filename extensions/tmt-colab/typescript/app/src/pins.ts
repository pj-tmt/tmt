import { ManagementError } from './management.js';
import { text } from './strings.js';

const t = text.pageList;

/** The words for a refused or failed pin, whichever surface asked for it. */
export function pinFailureText(failure: unknown): string {
  const code = failure instanceof ManagementError ? failure.code : 'UNKNOWN';
  if (code === 'CAPACITY') return t.pinFull;
  return t.pinFailures[code as keyof typeof t.pinFailures] ?? t.pinFailures.UNKNOWN;
}
