import { entryHash, entryMount, entryOpenTarget, publicEntry } from './entry.js';
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
import { ownerPageHint } from './reading-link.js';
import { record } from './storage.js';
import { shortPageId, validPagePrefix } from './short-links.js';
import { jsonResponse } from './registration.js';
import { WORK_TASKS, taskNumber } from './work.js';

import type { ExpiryInfo } from './expiry.js';

/** Project limits; `limits.rs` and the contract's Projects section hold the same numbers. */
export const PROJECT_POINTS = 32;
export const PROJECTS_PER_PAGE = 8;
/** Most names one space lists: creation stops at 256, and a space migrated from labels may hold up to its 1,000 assignments. */
const PROJECTS_PER_SPACE = 1000;

/** Page summary limits; the contract's Page summary section holds the same numbers. */
export const AUTHOR_BYTES = 128;
/** Assignees a list row names; `work.rs` (`ROW_WORKING`) holds the same number. */
export const ROW_WORKING = 3;
/** A row's assignee label, as long as the task's own (`work.ts`). */
const ASSIGNEE_BYTES = 128;
/** 200 proposals + 50 readings + 50 answer blocks + 50 reviews. */
export const NEEDS_YOU_MAX = 350;
const COMMENTS_MAX = 4_294_967_295;
/** The complete `/api/pages` reply: 1,000 pages with the longest labels, summaries, work rows and
 * parents (about 1.9 MB) fit. */
export const DISCOVERY_BYTES = 2 * 1024 * 1024;
/** The longest local page title the owner serves, in UTF-8 bytes (`PAGE_TITLE_BYTES` in `limits.rs`). */
export const PAGE_TITLE_BYTES = 512;

/** Project names are display-only text: unique, 1 to 32 code points, no control character. The owner enforces the full grammar; the browser refuses what it could not render safely. */
function projectNames(names: readonly unknown[]): boolean {
  return (
    new Set(names).size === names.length &&
    names.every(
      (name) =>
        typeof name === 'string' &&
        [...name].length >= 1 &&
        [...name].length <= PROJECT_POINTS &&
        !/\p{Cc}/u.test(name),
    )
  );
}

/** An assignee a list row names: no identity, only the display label. */
export interface RowAssignee {
  kind: 'agent' | 'person';
  name: string;
}
/** What a list row shows of a page's tasks, every task that follows a page showing that page's
 * stage; omitted for a page with no work (contract: Page summary). */
export interface PageWorkRow {
  stage: 'discussion' | 'in-progress' | 'done';
  done: number;
  total: number;
  /** Tasks waiting in review for a sign-off. */
  signoff: number;
  /** The assignees of in-progress tasks, each once in task order, at most `ROW_WORKING`. */
  working: readonly RowAssignee[];
  /** How many more assignees work, when more than `ROW_WORKING` do. */
  more?: number;
}
/** What the owner derives for the list: comment count, a proposal awaiting the owner, the creator. */
export interface PageActivity {
  comments: number;
  /** At least one undecided, unresolved proposal; in v1 the mark means "a proposal awaits you". */
  waiting: boolean;
  /** Pending proposals, readings, answer blocks and reviews; one count per target ID. */
  needsYou: number;
  /** The page's creation label (`meta.originalAuthor`); omitted when it has none. */
  author?: string;
  /** The page's tasks; omitted for a page with no work, so a fresh summary without it is in discussion. */
  work?: PageWorkRow;
  /** The task that follows this page; omitted when none does. */
  parent?: { page: string; task: string };
}
export interface PageInfo extends ExpiryInfo {
  pageId: string;
  epoch: string;
  sharing: 'private' | 'link' | 'public';
  history: 'shared' | 'current';
  archived: boolean;
  /** Display-only projects the page is in; sorted by the owner store, at most 8. */
  projects: readonly string[];
  /** Display-only and unsigned; absent until the owner has derived it for the page's current state. */
  summary?: PageActivity;
  /** The owner's local plaintext title, current for the page's state; absent until the owner has folded it
   * (or when the reply had no room for it): the browser then keeps its own hint. */
  title?: string;
}
export interface PageId {
  pageId: string;
  deleted: boolean;
}
export interface Bootstrap {
  space: string;
  owner: Uint8Array;
  revision: string;
  /** Every project of the space, including those with no page; sorted by the owner store. */
  projects: readonly string[];
  pages: PageInfo[];
  pageIds: PageId[];
}
/** A work row is untrusted text too: closed values, and counts that agree with the stage. */
function workRow(value: unknown): asserts value is PageWorkRow {
  requireValue(value !== null && typeof value === 'object' && !Array.isArray(value));
  exactKeys(
    value,
    'more' in value
      ? ['stage', 'done', 'total', 'signoff', 'working', 'more']
      : ['stage', 'done', 'total', 'signoff', 'working'],
  );
  const count = (n: unknown): n is number =>
    Number.isSafeInteger(n) && Number(n) >= 0 && Number(n) <= WORK_TASKS;
  requireValue(count(value.done) && count(value.total) && count(value.signoff));
  requireValue(value.done <= value.total && value.signoff <= value.total - value.done);
  // The stage follows the counts, as `waiting` follows `needsYou`.
  requireValue(
    value.stage ===
      (value.total === 0 ? 'discussion' : value.done === value.total ? 'done' : 'in-progress'),
  );
  requireValue(Array.isArray(value.working) && value.working.length <= ROW_WORKING);
  for (const who of value.working) {
    requireValue(who !== null && typeof who === 'object' && !Array.isArray(who));
    exactKeys(who, ['kind', 'name']);
    requireValue(
      (who.kind === 'agent' || who.kind === 'person') &&
        typeof who.name === 'string' &&
        who.name.length >= 1 &&
        new TextEncoder().encode(who.name).length <= ASSIGNEE_BYTES &&
        !/\p{Cc}/u.test(who.name),
    );
  }
  if ('more' in value)
    requireValue(
      count(value.more) &&
        value.more >= 1 &&
        value.working.length === ROW_WORKING &&
        value.working.length + value.more <= value.total,
    );
}
/** The owner's display-only summary is untrusted text: bounded and typed, never trusted for more. */
export function activity(value: unknown): asserts value is PageActivity {
  requireValue(value !== null && typeof value === 'object' && !Array.isArray(value));
  exactKeys(value, [
    'comments',
    'waiting',
    'needsYou',
    ...('author' in value ? ['author'] : []),
    ...('work' in value ? ['work'] : []),
    ...('parent' in value ? ['parent'] : []),
  ]);
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
  if ('work' in value) workRow(value.work);
  if ('parent' in value) {
    const parent = value.parent;
    requireValue(parent !== null && typeof parent === 'object' && !Array.isArray(parent));
    exactKeys(parent, ['page', 'task']);
    requireValue(typeof parent.page === 'string' && typeof parent.task === 'string');
    generatedId(parent.page);
    requireValue(taskNumber(parent.task) !== undefined);
  }
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
    DISCOVERY_BYTES,
  );
  exactKeys(value, ['spaceId', 'ownerKey', 'revision', 'projects', 'pages', 'pageIds']);
  requireValue(typeof value.spaceId === 'string' && typeof value.revision === 'string');
  spaceId(value.spaceId);
  decimal(value.revision);
  const owner = binary(value.ownerKey, 32, 32);
  requireValue((await deriveSpaceId(owner)) === value.spaceId);
  requireValue(Array.isArray(value.pages) && value.pages.length <= 1000);
  requireValue(
    Array.isArray(value.projects) &&
      value.projects.length <= PROJECTS_PER_SPACE &&
      projectNames(value.projects),
  );
  const known = new Set<unknown>(value.projects);
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
      'projects',
      ...(page !== null && typeof page === 'object' && 'summary' in page ? ['summary'] : []),
      ...(page !== null && typeof page === 'object' && 'title' in page ? ['title'] : []),
    ]);
    if ('summary' in page) activity(page.summary);
    // Untrusted display text: one line of plain text, as the owner's store keeps it.
    if ('title' in page)
      requireValue(
        typeof page.title === 'string' &&
          page.title.length >= 1 &&
          new TextEncoder().encode(page.title).length <= PAGE_TITLE_BYTES &&
          !/\p{Cc}/u.test(page.title),
      );
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
    // Projects are display-only text from the owner store; the owner enforces the full grammar, the
    // browser only refuses what it could not render safely or what the store never writes.
    requireValue(
      Array.isArray(page.projects) &&
        page.projects.length <= PROJECTS_PER_PAGE &&
        projectNames(page.projects) &&
        page.projects.every((name: unknown) => known.has(name)),
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
    // The location keeps only the canonical fragment of its display target: a plain #id becomes
    // #anchor=<id>, and a fragment that names none is dropped, so nothing downstream (a copied
    // link, an Ask's page link) carries it.
    if (publicEntry()) {
      const canonical = entryHash(entryOpenTarget());
      if (location.hash !== canonical)
        history.replaceState(history.state, '', location.pathname + location.search + canonical);
    }
    let path: string | null = null;
    if (!fragment && !publicEntry()) {
      const incoming = new URLSearchParams(location.hash.slice(1));
      path = incoming.get('path');
      requireValue(
        [...incoming.keys()].every((key) => key === 'path') && incoming.getAll('path').length <= 1,
      );
      // A short link's prefix, or a page's full ID from its native deep link (`<mount>/pages/<id>`).
      requireValue(
        path === null ||
          (path.startsWith('/short/') && validPagePrefix(path.slice(7))) ||
          path.startsWith('/pages/'),
      );
      if (path?.startsWith('/pages/')) generatedId(path.slice(7));
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
    const hint = ownerPageHint(new URL(location.href), value.spaceId);
    const ids = (value.pageIds as PageId[]).map((row) => row.pageId);
    let destination = '/colab/';
    if (target?.startsWith('/pages/')) {
      const page = target.slice(7);
      generatedId(page);
      destination = `/p/${shortPageId(page, ids)}`;
    } else if (target?.startsWith('/short/') && validPagePrefix(target.slice(7))) {
      destination = `/p/${target.slice(7)}`;
    } else requireValue(target === null || target === '/');
    // The owner's cosmetic reading or shell-anchor hint rides the short location.
    const tail =
      hint && destination !== '/colab/'
        ? 'read' in hint
          ? `#read=${hint.read}`
          : `#anchor=${hint.anchor}`
        : '';
    history.replaceState(history.state, '', destination + tail);
  }
  return {
    space: value.spaceId,
    owner,
    revision: value.revision,
    projects: value.projects as unknown as readonly string[],
    pages: value.pages as unknown as PageInfo[],
    pageIds: value.pageIds as unknown as PageId[],
  };
}
