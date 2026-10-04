import { binary, decimal, generatedId, requireValue, spaceId } from '@tmt/colab-client';

/** The colab-v1 reader link fragment, parsed once and never kept as a string afterwards. */
export interface ReaderLink {
  space: string;
  page: string;
  link: string;
  revision: string;
  statement: Uint8Array;
  seed: Uint8Array;
}
const KEYS = ['v', 'space', 'page', 'link', 'rev', 'st', 'seed'];

/** Strict grammar: `#v=1&space&page&link&rev&st&seed`, each key exactly once, canonical values. */
export function parseReaderFragment(hash: string): ReaderLink {
  requireValue(hash.startsWith('#') && hash.length <= 512);
  const seen = new Map<string, string>();
  for (const part of hash.slice(1).split('&')) {
    const at = part.indexOf('=');
    requireValue(at > 0 && !part.includes('%'));
    const key = part.slice(0, at);
    requireValue(KEYS.includes(key) && !seen.has(key));
    seen.set(key, part.slice(at + 1));
  }
  requireValue(seen.size === KEYS.length && seen.get('v') === '1');
  const get = (key: string) => seen.get(key)!;
  spaceId(get('space'));
  generatedId(get('page'));
  generatedId(get('link'));
  decimal(get('rev'));
  return {
    space: get('space'),
    page: get('page'),
    link: get('link'),
    revision: get('rev'),
    statement: binary(get('st'), 32, 32),
    seed: binary(get('seed'), 32, 32),
  };
}
