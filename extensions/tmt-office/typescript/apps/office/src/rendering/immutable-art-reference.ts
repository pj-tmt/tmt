import { indexedArtKey } from './indexed-art-contract.js';

const digest = /^sha256:[0-9a-f]{64}$/;

export function validImmutableArtReference(value: unknown): value is string {
  if (typeof value !== 'string') return false;
  const separator = value.indexOf('/');
  return (
    separator > 0 &&
    value.indexOf('/', separator + 1) === -1 &&
    digest.test(value.slice(0, separator)) &&
    indexedArtKey(value.slice(separator + 1))
  );
}

export function validImmutableArtDigest(value: unknown): value is string {
  return typeof value === 'string' && digest.test(value);
}
