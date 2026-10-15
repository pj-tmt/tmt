import {
  binary,
  decimal,
  deriveSpaceId,
  generatedId,
  requireValue,
  spaceId,
  type Bytes,
} from '@tmt/colab-client';

/** The colab-v1 reader link fragment, parsed once and never kept as a string afterwards. */
export interface ReaderLink {
  space: string;
  page: string;
  link: string;
  revision: string;
  statement: Uint8Array;
  seed: Uint8Array;
  /** The owner's root public key; it derives `space`. */
  root: Bytes;
  /** A hint for the single self-admission create; the capability comes from the signed link. */
  role: 'viewer' | 'commenter' | 'editor';
}
const KEYS = ['v', 'space', 'page', 'link', 'rev', 'st', 'seed', 'root', 'role'];
const ROLES = ['viewer', 'commenter', 'editor'];
const OLDER_KEYS = ['v', 'space', 'page', 'link', 'rev', 'st', 'seed'];

/** A well-formed link from before `root` and `role`: it opens nothing now, and the page says why. */
export function isOlderReaderFragment(hash: string): boolean {
  if (!hash.startsWith('#') || hash.length > 512) return false;
  const parts = hash.slice(1).split('&'),
    keys = parts.map((part) => part.split('=', 1)[0]);
  return (
    parts.length === OLDER_KEYS.length &&
    OLDER_KEYS.every((key) => keys.includes(key)) &&
    parts.includes('v=1')
  );
}

/** Strict grammar: `#v=1&space&page&link&rev&st&seed&root&role`, each key exactly once, canonical
 * values, and a `root` that derives `space`. */
export async function parseReaderFragment(hash: string): Promise<ReaderLink> {
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
  const root = binary(get('root'), 32, 32);
  requireValue((await deriveSpaceId(root)) === get('space') && ROLES.includes(get('role')));
  return {
    space: get('space'),
    page: get('page'),
    link: get('link'),
    revision: get('rev'),
    statement: binary(get('st'), 32, 32),
    seed: binary(get('seed'), 32, 32),
    root,
    role: get('role') as ReaderLink['role'],
  };
}
