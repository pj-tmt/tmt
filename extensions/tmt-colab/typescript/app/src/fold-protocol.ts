import { exactKeys, generatedId, requireValue, text } from '@tmt/colab-client';
/** Plaintext-only decoder protocol. No CryptoKeys or transport capabilities. */
export const SOURCE_BYTES = 2 * 1024 * 1024;
export const UPDATE_BYTES = 256 * 1024;
export const BASELINE_UPDATE_BYTES = SOURCE_BYTES + UPDATE_BYTES + 1024;
export const STATE_BYTES = 4 * 1024 * 1024;
export type FoldCommand =
  | { type: 'apply' | 'check'; updates: Uint8Array[]; own?: OwnUpdate[] }
  | { type: 'checkpoint'; update: Uint8Array; writer?: string }
  | { type: 'prepare'; source: string; base?: string }
  | {
      type: 'baseline';
      update: Uint8Array;
      title: string;
      sourceDigest: Uint8Array;
      commitment: Uint8Array;
    };
export interface Projection {
  source: string;
  title: string;
}
export interface OwnUpdate {
  writer: string;
  update: Uint8Array;
}
export interface AdmittedUpdate extends OwnUpdate {
  namespace: 'content' | 'own';
}
export type JsonValue =
  | null
  | boolean
  | number
  | string
  | JsonValue[]
  | { [key: string]: JsonValue };
export interface OwnProjection {
  threads: Record<string, JsonValue>;
  messages: Record<string, JsonValue>;
  intents: Record<string, JsonValue>;
  replies: Record<string, JsonValue>;
}
export type OwnState = Record<string, OwnProjection>;
export function validateOwn(value: unknown): asserts value is OwnState {
  requireValue(value !== null && typeof value === 'object' && !Array.isArray(value));
  requireValue(text(JSON.stringify(value)).length <= STATE_BYTES);
  const pending: unknown[] = [value];
  while (pending.length) {
    const item = pending.pop();
    if (item === null || typeof item === 'boolean') continue;
    if (typeof item === 'string') {
      text(item);
      continue;
    }
    if (typeof item === 'number') {
      requireValue(Number.isFinite(item));
      continue;
    }
    requireValue(item !== undefined && typeof item === 'object');
    if (Array.isArray(item)) for (const child of item) pending.push(child);
    else {
      requireValue(
        Object.getPrototypeOf(item) === Object.prototype || Object.getPrototypeOf(item) === null,
      );
      for (const child of Object.values(item)) pending.push(child);
    }
  }
  let threads = 0;
  const entries = Object.entries(value);
  requireValue(entries.length <= 256);
  for (const [writer, roots] of entries) {
    generatedId(writer);
    exactKeys(roots, ['threads', 'messages', 'intents', 'replies']);
    for (const map of Object.values(roots))
      requireValue(map !== null && typeof map === 'object' && !Array.isArray(map));
    const projection = roots as unknown as OwnProjection;
    threads += Object.keys(projection.threads).length;
    for (const message of Object.values(projection.messages)) {
      if (message !== null && typeof message === 'object' && Object.hasOwn(message, 'body')) {
        const body = (message as Record<string, unknown>).body;
        requireValue(typeof body === 'string' && text(body).length <= 16 * 1024);
      }
    }
  }
  requireValue(threads <= 1000);
}
export interface FoldResult extends Projection {
  own: OwnState;
  update: Uint8Array;
}
export function validateProjection(value: unknown): asserts value is Projection {
  if (!value || typeof value !== 'object') throw new Error('Invalid decoder projection');
  const { source, title } = value as Projection;
  if (
    typeof source !== 'string' ||
    typeof title !== 'string' ||
    text(source).length > SOURCE_BYTES ||
    text(title).length > UPDATE_BYTES
  )
    throw new Error('Invalid decoder projection');
}
