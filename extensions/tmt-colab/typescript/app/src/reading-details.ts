import { exactKeys, requireValue } from '@tmt/colab-client';
import { activity, type PageActivity } from './bootstrap.js';
import { validReadingId } from './reading-link.js';
import { jsonResponse } from './registration.js';

export const READING_DETAILS_BYTES = 192 * 1024;
export const READING_DETAILS_PAGES = 20;
export const NEEDS_PASSAGES = 20;
export const LATEST_POINTS = 160;
export interface ReadingDescriptor {
  id: string;
  title: string;
  agent: string;
  why: string;
  state: 'unread' | 'read';
}
export type NeedKind = 'review' | 'question' | 'reading' | 'proposal';
/** One pending item the page's count includes: `id` is the pending-target ID (a question's is its
 * group's). `why` is a reading's reason only. */
export interface NeedDescriptor {
  kind: NeedKind;
  id: string;
  title: string;
  agent?: string;
  why?: string;
}
/** The page's newest comment as an excerpt; `who` is a display-only device label. */
export interface LatestComment {
  who?: string;
  atMs: number;
  text: string;
}
export type ReadingDetails =
  | {
      pageId: string;
      state: 'ready';
      fingerprint: string;
      summary: PageActivity;
      readings: ReadingDescriptor[];
      /** The first pending items by rank (review, question, reading, proposal); the count stays `summary.needsYou`. */
      needs: NeedDescriptor[];
      latest?: LatestComment;
    }
  | { pageId: string; state: 'pending' | 'unavailable' };

function text(value: unknown, limit: number): asserts value is string {
  requireValue(
    typeof value === 'string' &&
      value.trim().length > 0 &&
      [...value].length <= limit &&
      !/\p{Cc}/u.test(value),
  );
}

/** Strict display-only metadata: a malformed response never becomes an empty ready result. */
export function readingDetails(value: unknown, pageId: string): ReadingDetails {
  requireValue(value !== null && typeof value === 'object' && !Array.isArray(value));
  requireValue(
    'state' in value && 'pageId' in value && value.pageId === pageId && validReadingId(pageId),
  );
  if (value.state === 'pending' || value.state === 'unavailable') {
    exactKeys(value, ['pageId', 'state']);
    return { pageId, state: value.state };
  }
  requireValue(value.state === 'ready');
  return completeDetails(value, pageId);
}

const NEED_KINDS: readonly NeedKind[] = ['review', 'question', 'reading', 'proposal'];

function need(item: unknown): NeedDescriptor {
  requireValue(item !== null && typeof item === 'object' && !Array.isArray(item));
  const optional = ['agent', 'why'].filter((key) => key in item);
  exactKeys(item, ['kind', 'id', 'title', ...optional]);
  requireValue(NEED_KINDS.includes(item.kind as NeedKind));
  requireValue(typeof item.id === 'string' && validReadingId(item.id));
  text(item.title, 200);
  if ('agent' in item) text(item.agent, 64);
  if ('why' in item) {
    requireValue(item.kind === 'reading');
    text(item.why, 200);
  }
  return {
    kind: item.kind as NeedKind,
    id: item.id,
    title: item.title,
    ...('agent' in item ? { agent: item.agent as string } : {}),
    ...('why' in item ? { why: item.why as string } : {}),
  };
}

function latestComment(value: unknown): LatestComment {
  requireValue(value !== null && typeof value === 'object' && !Array.isArray(value));
  exactKeys(value, ['atMs', 'text', ...('who' in value ? ['who'] : [])]);
  if ('who' in value) text(value.who, 64);
  text(value.text, LATEST_POINTS);
  requireValue(Number.isSafeInteger(value.atMs) && (value.atMs as number) >= 0);
  return {
    ...('who' in value ? { who: value.who as string } : {}),
    atMs: value.atMs as number,
    text: value.text as string,
  };
}

function completeDetails(value: unknown, pageId: string): ReadingDetails {
  requireValue(value !== null && typeof value === 'object' && !Array.isArray(value));
  exactKeys(value, [
    'pageId',
    'state',
    'fingerprint',
    'summary',
    'readings',
    'needs',
    ...('latest' in value ? ['latest'] : []),
  ]);
  requireValue(typeof value.fingerprint === 'string' && /^[0-9a-f]{64}$/.test(value.fingerprint));
  activity(value.summary);
  requireValue(Array.isArray(value.readings) && value.readings.length <= 50);
  const ids = new Set<string>();
  const readings: ReadingDescriptor[] = [];
  for (const item of value.readings) {
    exactKeys(item, ['id', 'title', 'agent', 'why', 'state']);
    requireValue(typeof item.id === 'string' && validReadingId(item.id) && !ids.has(item.id));
    ids.add(item.id);
    text(item.title, 200);
    text(item.agent, 64);
    text(item.why, 200);
    requireValue(item.state === 'unread' || item.state === 'read');
    readings.push({
      id: item.id,
      title: item.title,
      agent: item.agent,
      why: item.why,
      state: item.state,
    });
  }
  requireValue(Array.isArray(value.needs) && value.needs.length <= NEEDS_PASSAGES);
  const needs = value.needs.map(need);
  requireValue(new Set(needs.map((item) => item.id)).size === needs.length);
  return {
    pageId,
    state: 'ready',
    fingerprint: value.fingerprint,
    summary: value.summary,
    readings,
    needs,
    ...('latest' in value ? { latest: latestComment(value.latest) } : {}),
  };
}

/** Owner-mounted HTTP only. Hosted/member adapters do not expose this optional port. */
export async function readReadingDetails(
  mount: URL,
  pageId: string,
  signal: AbortSignal,
): Promise<ReadingDetails> {
  requireValue(validReadingId(pageId));
  signal.throwIfAborted();
  const response = await fetch(new URL(`api/pages/${pageId}/summary-details`, mount), {
    credentials: 'same-origin',
    cache: 'no-store',
    signal: AbortSignal.any([signal, AbortSignal.timeout(10_000)]),
  });
  return readingDetails(await jsonResponse(response, READING_DETAILS_BYTES), pageId);
}

type Read = (id: string, signal: AbortSignal) => Promise<ReadingDetails>;

/** One shell opening. At most `READING_DETAILS_PAGES` distinct pages are read, one at a time;
 * a failed or unavailable read still counts, so a reader never fans out beyond the cap. */
export class ReadingDetailsQueue {
  #lifetime = new AbortController();
  #selected = new Set<string>();
  #pending: string[] = [];
  #running: Promise<void> | undefined;
  constructor(
    private readonly read: Read,
    private readonly publish: (id: string, details: ReadingDetails) => void,
  ) {}
  enqueue(pageIds: readonly string[]): void {
    if (this.#lifetime.signal.aborted) return;
    for (const id of pageIds) {
      if (this.#selected.has(id)) continue;
      if (this.#selected.size === READING_DETAILS_PAGES) break;
      this.#selected.add(id);
      this.#pending.push(id);
    }
    this.#start();
  }
  #start(): void {
    if (this.#running || !this.#pending.length || this.#lifetime.signal.aborted) return;
    this.#running = this.#drain().finally(() => {
      this.#running = undefined;
      this.#start();
    });
  }
  async #drain(): Promise<void> {
    while (this.#pending.length && !this.#lifetime.signal.aborted) {
      const id = this.#pending.shift()!;
      try {
        const details = await this.read(id, this.#lifetime.signal);
        if (!this.#lifetime.signal.aborted) this.publish(id, details);
      } catch {
        // Optional details stay unknown. A failed attempt still consumes this generation's cap.
      }
    }
  }
  close(): Promise<void> {
    this.#lifetime.abort();
    this.#pending = [];
    return this.#running ?? Promise.resolve();
  }
}
