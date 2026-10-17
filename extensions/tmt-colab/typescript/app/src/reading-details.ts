import { exactKeys, requireValue } from '@tmt/colab-client';
import { activity, type PageActivity } from './bootstrap.js';
import { validReadingId } from './reading-link.js';
import { jsonResponse } from './registration.js';

export const READING_DETAILS_BYTES = 128 * 1024;
export const READING_DETAILS_PAGES = 20;
export interface ReadingDescriptor {
  id: string;
  title: string;
  agent: string;
  why: string;
  state: 'unread' | 'read';
}
export type ReadingDetails =
  | {
      pageId: string;
      state: 'ready';
      fingerprint: string;
      summary: PageActivity;
      readings: ReadingDescriptor[];
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

function completeDetails(value: unknown, pageId: string): ReadingDetails {
  exactKeys(value, ['pageId', 'state', 'fingerprint', 'summary', 'readings']);
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
  return {
    pageId,
    state: 'ready',
    fingerprint: value.fingerprint,
    summary: value.summary,
    readings,
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

type Chapter = { id: string; summary?: { needsYou: number } };
type Read = (id: string, signal: AbortSignal) => Promise<ReadingDetails>;

/** One Home generation. Only displayed chapters with a known positive count consume reads. */
export class HomeReadingDetails {
  #lifetime = new AbortController();
  #selected = new Set<string>();
  #pending: string[] = [];
  #running: Promise<void> | undefined;
  constructor(
    private readonly read: Read,
    private readonly publish: (id: string, details: ReadingDetails) => void,
  ) {}
  enqueue(chapters: readonly Chapter[]): void {
    if (this.#lifetime.signal.aborted) return;
    for (const chapter of chapters) {
      if (!chapter.summary || chapter.summary.needsYou <= 0 || this.#selected.has(chapter.id))
        continue;
      if (this.#selected.size === READING_DETAILS_PAGES) break;
      this.#selected.add(chapter.id);
      this.#pending.push(chapter.id);
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
