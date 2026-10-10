import { entryMount, entryThread, publicEntry } from './entry.js';
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
import { shortPageId, validPagePrefix } from './short-links.js';
import { jsonResponse } from './registration.js';

import type { ExpiryInfo } from './expiry.js';

export interface PageInfo extends ExpiryInfo {
  pageId: string;
  epoch: string;
  sharing: 'private' | 'link' | 'public';
  history: 'shared' | 'current';
  archived: boolean;
}
export interface PageId {
  pageId: string;
  deleted: boolean;
}
export interface Bootstrap {
  space: string;
  owner: Uint8Array;
  revision: string;
  pages: PageInfo[];
  pageIds: PageId[];
}
export function mountUrl(): URL {
  if (publicEntry()) return entryMount();
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
    512 * 1024,
  );
  exactKeys(value, ['spaceId', 'ownerKey', 'revision', 'pages', 'pageIds']);
  requireValue(typeof value.spaceId === 'string' && typeof value.revision === 'string');
  spaceId(value.spaceId);
  decimal(value.revision);
  const owner = binary(value.ownerKey, 32, 32);
  requireValue((await deriveSpaceId(owner)) === value.spaceId);
  requireValue(Array.isArray(value.pages) && value.pages.length <= 1000);
  let previous = '';
  for (const page of value.pages) {
    exactKeys(page, [
      'pageId',
      'epoch',
      'sharing',
      'history',
      'archived',
      'retentionDays',
      'lastUpdateAtMs',
      'expiresAtMs',
      'warnings',
    ]);
    requireValue(
      page.retentionDays === null ||
        (Number.isSafeInteger(page.retentionDays) && Number(page.retentionDays) > 0),
    );
    for (const key of ['lastUpdateAtMs', 'expiresAtMs'])
      requireValue(
        page[key] === null || (Number.isSafeInteger(page[key]) && Number(page[key]) >= 0),
      );
    requireValue(
      Array.isArray(page.warnings) &&
        page.warnings.length <= 1 &&
        page.warnings.every(
          (w) =>
            typeof w === 'string' &&
            ['expiry-unavailable', 'expiry-out-of-range', 'expires-soon', 'expired'].includes(w),
        ),
    );
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
  requireValue(Array.isArray(value.pageIds) && value.pageIds.length <= 1000);
  let previousId = '';
  const liveIds: string[] = [];
  for (const page of value.pageIds) {
    exactKeys(page, ['pageId', 'deleted']);
    requireValue(typeof page.pageId === 'string' && typeof page.deleted === 'boolean');
    generatedId(page.pageId);
    requireValue(page.pageId > previousId);
    previousId = page.pageId;
    if (!page.deleted) liveIds.push(page.pageId);
  }
  requireValue(JSON.stringify(liveIds) === JSON.stringify(value.pages.map((page) => page.pageId)));
  await verify?.(value.spaceId, owner);
  await navigator.locks.request(`colab-pin:${mount.href}`, async () => {
    const pin = await record<{ space: string; owner: Uint8Array }>(`pin:${mount.href}`),
      fragment = new URLSearchParams(location.hash.slice(1)).get('space');
    requireValue(
      (!fragment || fragment === value.spaceId) &&
        (!pin || (pin.space === value.spaceId && equal(pin.owner, owner))),
    );
    if (publicEntry()) entryThread();
    let path: string | null = null;
    if (!fragment && !publicEntry()) {
      const incoming = new URLSearchParams(location.hash.slice(1));
      path = incoming.get('path');
      requireValue(
        [...incoming.keys()].every((key) => key === 'path') && incoming.getAll('path').length <= 1,
      );
      requireValue(path === null || (path.startsWith('/short/') && validPagePrefix(path.slice(7))));
    }
    if (!pin) await record(`pin:${mount.href}`, { space: value.spaceId, owner });
    if (!fragment && !publicEntry()) {
      history.replaceState(
        history.state,
        '',
        `${mount.pathname}#space=${value.spaceId}${path ? `&path=${encodeURIComponent(path)}` : ''}`,
      );
    }
  });
  if (
    !publicEntry() &&
    typeof location.pathname === 'string' &&
    location.pathname.startsWith('/r/')
  ) {
    const target = new URLSearchParams(location.hash.slice(1)).get('path');
    const ids = (value.pageIds as PageId[]).map((row) => row.pageId);
    let destination = '/colab/';
    if (target?.startsWith('/pages/')) {
      const page = target.slice(7);
      generatedId(page);
      destination = `/p/${shortPageId(page, ids)}`;
    } else if (target?.startsWith('/short/') && validPagePrefix(target.slice(7))) {
      destination = `/p/${target.slice(7)}`;
    } else requireValue(target === null || target === '/');
    history.replaceState(history.state, '', destination);
  }
  return {
    space: value.spaceId,
    owner,
    revision: value.revision,
    pages: value.pages as unknown as PageInfo[],
    pageIds: value.pageIds as unknown as PageId[],
  };
}
