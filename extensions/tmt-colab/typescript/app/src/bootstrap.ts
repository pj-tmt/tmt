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
import { validPagePrefix } from './short-links.js';
import { jsonResponse } from './registration.js';

import type { ExpiryInfo } from './expiry.js';

/** Page label limits; `limits.rs` and the contract's Page labels section hold the same numbers. */
export const LABEL_POINTS = 32;
export const LABELS_PER_PAGE = 8;

/** Page summary limits; the contract's Page summary section holds the same numbers. */
export const AUTHOR_BYTES = 128;
/** 200 proposals + 50 readings + 50 answer blocks + 50 reviews. */
export const NEEDS_YOU_MAX = 350;
const COMMENTS_MAX = 4_294_967_295;
/** Home summary limits; `limits.rs` and the contract's Home summary section hold the same numbers. */
export const HOME_SUMMARY_FIRST_POINTS = 24;
export const HOME_SUMMARY_SECOND_POINTS = 60;
/** The complete `/api/pages` reply: 1,000 pages with the longest labels and summaries (about 780 KB) fit. */
export const DISCOVERY_BYTES = 1024 * 1024;

/** What the owner derives for the list: comment count, a proposal awaiting the owner, the creator. */
export interface PageActivity {
  comments: number;
  /** At least one undecided, unresolved proposal; in v1 the mark means "a proposal awaits you". */
  waiting: boolean;
  /** Pending proposals, readings, answer blocks and reviews; one count per target ID. */
  needsYou: number;
  /** The page's creation label (`meta.originalAuthor`); omitted when it has none. */
  author?: string;
}
/** Two display-only lines under Today's greeting, written by the space's maintainer agent. */
export interface HomeSummary {
  readonly lines: readonly [string, string];
  /** The writer's display name; omitted when the writer had no agent identity. Never proof. */
  readonly author?: string;
  /** The owner's clock at the write. */
  readonly atMs: number;
}
export interface PageInfo extends ExpiryInfo {
  pageId: string;
  epoch: string;
  sharing: 'private' | 'link' | 'public';
  history: 'shared' | 'current';
  archived: boolean;
  /** Display-only names the owner organizes pages with; sorted by the owner store, at most 8. */
  labels: readonly string[];
  /** Display-only and unsigned; absent until the owner has derived it for the page's current state. */
  summary?: PageActivity;
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
  homeSummary?: HomeSummary;
}
/** The owner's display-only summary is untrusted text: bounded and typed, never trusted for more. */
export function activity(value: unknown): asserts value is PageActivity {
  requireValue(value !== null && typeof value === 'object' && !Array.isArray(value));
  exactKeys(
    value,
    'author' in value
      ? ['comments', 'waiting', 'needsYou', 'author']
      : ['comments', 'waiting', 'needsYou'],
  );
  requireValue(
    Number.isSafeInteger(value.comments) &&
      Number(value.comments) >= 0 &&
      Number(value.comments) <= COMMENTS_MAX &&
      typeof value.waiting === 'boolean' &&
      Number.isSafeInteger(value.needsYou) &&
      Number(value.needsYou) >= 0 &&
      Number(value.needsYou) <= NEEDS_YOU_MAX &&
      value.waiting === Number(value.needsYou) > 0,
  );
  if ('author' in value)
    requireValue(
      typeof value.author === 'string' &&
        value.author.length >= 1 &&
        new TextEncoder().encode(value.author).length <= AUTHOR_BYTES &&
        !/\p{Cc}/u.test(value.author),
    );
}
/** The owner enforces the full grammar; the browser refuses what it could not render safely. */
export function homeSummary(value: unknown): asserts value is HomeSummary {
  requireValue(value !== null && typeof value === 'object' && !Array.isArray(value));
  exactKeys(value, 'author' in value ? ['lines', 'author', 'atMs'] : ['lines', 'atMs']);
  const limits = [HOME_SUMMARY_FIRST_POINTS, HOME_SUMMARY_SECOND_POINTS];
  requireValue(
    Array.isArray(value.lines) &&
      value.lines.length === limits.length &&
      value.lines.every(
        (line, index) =>
          typeof line === 'string' &&
          [...line].length >= 1 &&
          [...line].length <= limits[index] &&
          !/\p{Cc}/u.test(line),
      ) &&
      Number.isSafeInteger(value.atMs) &&
      Number(value.atMs) >= 0,
  );
  if ('author' in value)
    requireValue(
      typeof value.author === 'string' &&
        value.author.length >= 1 &&
        new TextEncoder().encode(value.author).length <= AUTHOR_BYTES &&
        !/\p{Cc}/u.test(value.author),
    );
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
    DISCOVERY_BYTES,
  );
  exactKeys(value, [
    'spaceId',
    'ownerKey',
    'revision',
    'pages',
    'pageIds',
    ...(value !== null && typeof value === 'object' && 'homeSummary' in value
      ? ['homeSummary']
      : []),
  ]);
  if ('homeSummary' in value) homeSummary(value.homeSummary);
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
      'labels',
      ...(page !== null && typeof page === 'object' && 'summary' in page ? ['summary'] : []),
    ]);
    if ('summary' in page) activity(page.summary);
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
          (w) => typeof w === 'string' && ['expires-soon', 'not-in-cloud'].includes(w),
        ),
    );
    // Labels are display-only text from the owner store; the owner enforces the full grammar, the
    // browser only refuses what it could not render safely or what the store never writes.
    requireValue(
      Array.isArray(page.labels) &&
        page.labels.length <= LABELS_PER_PAGE &&
        new Set(page.labels).size === page.labels.length &&
        page.labels.every(
          (label) =>
            typeof label === 'string' &&
            [...label].length >= 1 &&
            [...label].length <= LABEL_POINTS &&
            !/\p{Cc}/u.test(label),
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
    let path: string | null = null;
    if (!fragment) {
      const incoming = new URLSearchParams(location.hash.slice(1));
      path = incoming.get('path');
      requireValue(
        [...incoming.keys()].every((key) => key === 'path') && incoming.getAll('path').length <= 1,
      );
      requireValue(path === null || (path.startsWith('/short/') && validPagePrefix(path.slice(7))));
    }
    if (!pin) await record(`pin:${mount.href}`, { space: value.spaceId, owner });
    if (!fragment) {
      history.replaceState(
        history.state,
        '',
        `${mount.pathname}#space=${value.spaceId}${path ? `&path=${encodeURIComponent(path)}` : ''}`,
      );
    }
  });
  return {
    space: value.spaceId,
    owner,
    revision: value.revision,
    pages: value.pages as unknown as PageInfo[],
    pageIds: value.pageIds as unknown as PageId[],
    ...('homeSummary' in value ? { homeSummary: value.homeSummary as HomeSummary } : {}),
  };
}
