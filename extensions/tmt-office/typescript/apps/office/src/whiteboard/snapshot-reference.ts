import { uuidIdentifier } from '../contracts/record.js';

const PREFIX = 'tmt:whiteboard:snapshot:';

/** Local immutable locator; never a browser session URL or bearer capability. */
export function resolveSnapshotReference(value: string): string {
  return uuidIdentifier(value.startsWith(PREFIX) ? value.slice(PREFIX.length) : value);
}

export function snapshotReference(id: string): string {
  return `${PREFIX}${uuidIdentifier(id)}`;
}
