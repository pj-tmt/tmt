import {
  binary,
  decimal,
  deriveSpaceId,
  equal,
  exactKeys,
  generatedId,
  requireValue,
  spaceId,
} from '@tmt/colab-client';
import { record } from './storage.js';
import { jsonResponse } from './registration.js';

export interface PageInfo {
  pageId: string;
  epoch: string;
  sharing: 'private' | 'link' | 'public';
  history: 'shared' | 'current';
  archived: boolean;
}
export interface Bootstrap {
  space: string;
  owner: Uint8Array;
  revision: string;
  pages: PageInfo[];
}
export function mountUrl(): URL {
  const url = new URL(location.href);
  requireValue(/^\/r\/[a-z0-9]+\/x\/colab\/$/.test(url.pathname) && url.search === '');
  url.hash = '';
  return url;
}
/** Owner-only paired mount discovery is the sole TOFU exception. Later discovery
 * must match the durable origin/mount pin and fragment, with no silent re-pin. */
export async function discover(
  mount: URL,
  verify?: (space: string, owner: Uint8Array) => Promise<unknown>,
  signal?: AbortSignal,
): Promise<Bootstrap> {
  const value = await jsonResponse(
    await fetch(new URL('api/pages', mount), {
      signal: signal
        ? AbortSignal.any([signal, AbortSignal.timeout(10_000)])
        : AbortSignal.timeout(10_000),
    }),
    256 * 1024,
  );
  exactKeys(value, ['spaceId', 'ownerKey', 'revision', 'pages']);
  requireValue(typeof value.spaceId === 'string' && typeof value.revision === 'string');
  spaceId(value.spaceId);
  decimal(value.revision);
  const owner = binary(value.ownerKey, 32, 32);
  requireValue((await deriveSpaceId(owner)) === value.spaceId);
  requireValue(Array.isArray(value.pages) && value.pages.length <= 1000);
  let previous = '';
  for (const page of value.pages) {
    exactKeys(page, ['pageId', 'epoch', 'sharing', 'history', 'archived']);
    requireValue(typeof page.pageId === 'string' && typeof page.epoch === 'string');
    generatedId(page.pageId);
    decimal(page.epoch);
    requireValue(
      page.pageId > previous &&
        ['private', 'link', 'public'].includes(String(page.sharing)) &&
        ['shared', 'current'].includes(String(page.history)) &&
        typeof page.archived === 'boolean',
    );
    previous = page.pageId;
  }
  await verify?.(value.spaceId, owner);
  await navigator.locks.request(`colab-pin:${mount.href}`, async () => {
    const pin = await record<{ space: string; owner: Uint8Array }>(`pin:${mount.href}`),
      fragment = new URLSearchParams(location.hash.slice(1)).get('space');
    requireValue(
      (!fragment || fragment === value.spaceId) &&
        (!pin || (pin.space === value.spaceId && equal(pin.owner, owner))),
    );
    if (!pin) await record(`pin:${mount.href}`, { space: value.spaceId, owner });
    if (!fragment)
      history.replaceState(history.state, '', `${mount.pathname}#space=${value.spaceId}`);
  });
  return {
    space: value.spaceId,
    owner,
    revision: value.revision,
    pages: value.pages as unknown as PageInfo[],
  };
}
