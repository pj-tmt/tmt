import { text } from '@tmt/colab-client';
/** Plaintext-only decoder protocol. No CryptoKeys or transport capabilities. */
export const SOURCE_BYTES = 2 * 1024 * 1024;
export const UPDATE_BYTES = 256 * 1024;
export const STATE_BYTES = 4 * 1024 * 1024;
export type FoldCommand =
  | { type: 'apply' | 'check'; updates: Uint8Array[] }
  | { type: 'prepare'; source: string; base?: string };
export interface Projection {
  source: string;
  title: string;
}
export interface FoldResult extends Projection {
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
