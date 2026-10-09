import { decodeText, exactKeys, requireValue, text } from '@tmt/colab-client';
import type { ComposerEdit } from './components/message-composer-edit.js';
import { draftKey } from './keyring.js';
import { open, seal, type Sealed } from './local-seal.js';
import { MESSAGE_RECIPIENT_LIMIT } from './message-recipient.js';
import { record } from './storage.js';
import { COMMENT_BYTES } from './thread-records.js';

/** Unsent drafts kept per page; the oldest is dropped first. */
export const DRAFTS_PER_PAGE = 32;
const TARGET_BYTES = 64 * 1024;
/** Bounds one page's decrypted record, which is always read and written whole. */
const RECORD_BYTES = 3 * 1024 * 1024;
const WRITE_DELAY_MS = 400;

/**
 * Device-local persistence behind the page's in-memory drafts. A target is the owner's
 * draft key (`writer:id` for a thread, the quote selector JSON, or `chat`). Persisted
 * bytes are never sent, authorize nothing and are not a second committed-comment model.
 */
export interface DraftStore {
  /** The saved drafts, or `undefined` when device storage cannot be read. */
  load(page: string): Promise<ReadonlyMap<string, ComposerEdit> | undefined>;
  /** Applies the changes (`null` removes) on top of what is stored. True only once durable. */
  apply(page: string, changes: ReadonlyMap<string, ComposerEdit | null>): Promise<boolean>;
}

/** A nonblank, deliberately edited message is a draft; a creator seed alone is not. */
export function persistable(edit: ComposerEdit): boolean {
  return edit.edited !== false && edit.value.trim() !== '';
}

function validEdit(value: unknown): ComposerEdit {
  requireValue(value !== null && typeof value === 'object' && !Array.isArray(value));
  const fields = value as Record<string, unknown>;
  requireValue(
    Reflect.ownKeys(fields).every((k) => k === 'value' || k === 'mentions' || k === 'edited'),
  );
  const body = fields.value;
  requireValue(typeof body === 'string' && text(body).length <= COMMENT_BYTES);
  const edit: ComposerEdit = { value: body as string };
  if (Object.hasOwn(fields, 'edited')) {
    requireValue(typeof fields.edited === 'boolean');
    edit.edited = fields.edited as boolean;
  }
  if (Object.hasOwn(fields, 'mentions')) {
    const mentions = fields.mentions;
    requireValue(Array.isArray(mentions) && mentions.length <= MESSAGE_RECIPIENT_LIMIT);
    edit.mentions = (mentions as unknown[]).map((mention) => {
      exactKeys(mention, ['key', 'range']);
      exactKeys(mention.key, ['machine', 'agent']);
      exactKeys(mention.range, ['start', 'end']);
      const { machine, agent } = mention.key as Record<string, unknown>;
      const { start, end } = mention.range as Record<string, unknown>;
      requireValue(typeof machine === 'string' && machine.length > 0 && machine.length <= 64);
      requireValue(typeof agent === 'string' && agent.length > 0 && agent.length <= 64);
      requireValue(Number.isSafeInteger(start) && Number.isSafeInteger(end));
      requireValue((start as number) >= 0 && (start as number) < (end as number));
      requireValue((end as number) <= (body as string).length);
      return {
        key: { machine: machine as string, agent: agent as string },
        range: { start: start as number, end: end as number },
      };
    });
  }
  return edit;
}

/** A strict, ordered decode. Any malformed entry rejects the whole record. */
function decodeRecord(bytes: Uint8Array): Map<string, ComposerEdit> {
  const parsed: unknown = JSON.parse(decodeText(bytes));
  exactKeys(parsed, ['v', 'drafts']);
  const { v, drafts } = parsed as { v: unknown; drafts: unknown };
  requireValue(v === 1 && Array.isArray(drafts) && drafts.length <= DRAFTS_PER_PAGE);
  const result = new Map<string, ComposerEdit>();
  for (const entry of drafts as unknown[]) {
    exactKeys(entry, ['target', 'edit']);
    const { target, edit } = entry as { target: unknown; edit: unknown };
    requireValue(typeof target === 'string' && target.length > 0);
    requireValue(text(target as string).length <= TARGET_BYTES && !result.has(target as string));
    result.set(target as string, validEdit(edit));
  }
  return result;
}

function encodeRecord(drafts: ReadonlyMap<string, ComposerEdit>): Uint8Array<ArrayBuffer> {
  return text(
    JSON.stringify({
      v: 1,
      drafts: [...drafts].map(([target, edit]) => ({ target, edit })),
    }),
  );
}

/** The newest drafts that fit the per-page count and byte bounds. */
function bounded(drafts: Map<string, ComposerEdit>): Map<string, ComposerEdit> {
  while (
    drafts.size > DRAFTS_PER_PAGE ||
    (drafts.size > 1 && encodeRecord(drafts).length > RECORD_BYTES)
  )
    drafts.delete(drafts.keys().next().value!);
  return drafts;
}

/** Encrypts one record per space, device and page with the device-local draft key. */
export class LocalDraftStore implements DraftStore {
  constructor(
    readonly space: string,
    readonly device: string,
  ) {}
  #scope(page: string) {
    return ['tmt-colab-drafts-v1', this.space, this.device, page];
  }
  #record(page: string) {
    return `drafts:${this.space}:${this.device}:${page}`;
  }
  async #read(page: string): Promise<Map<string, ComposerEdit>> {
    const stored = await record<Sealed>(this.#record(page));
    if (stored === undefined) return new Map();
    try {
      return decodeRecord(
        await open(await draftKey(this.device), this.#scope(page), stored, RECORD_BYTES),
      );
    } catch {
      // A corrupt, foreign-scope or unknown-version record has no usable draft; the next
      // write replaces it. Key and storage faults surface from the caller's own reads.
      return new Map();
    }
  }
  async load(page: string) {
    try {
      return await this.#read(page);
    } catch {
      return undefined;
    }
  }
  async apply(page: string, changes: ReadonlyMap<string, ComposerEdit | null>) {
    try {
      return await navigator.locks.request(`colab-drafts:${this.#record(page)}`, async () => {
        const drafts = await this.#read(page);
        for (const [target, edit] of changes) {
          // An update moves to the newest position so the oldest is the one dropped.
          drafts.delete(target);
          if (edit !== null) drafts.set(target, validEdit(edit));
        }
        const sealed = await seal(
          await draftKey(this.device),
          this.#scope(page),
          encodeRecord(bounded(drafts)),
        );
        await record(this.#record(page), sealed);
        return true;
      });
    } catch {
      return false;
    }
  }
}

/**
 * One page's coalesced writes. `set` records the latest value per target; `flush` stores
 * them. A failed flush keeps the changes queued so a later one retries, never claiming
 * durability it did not confirm.
 */
export class DraftSession {
  #pending = new Map<string, ComposerEdit | null>();
  /** Targets stored or queued for storage, so clearing an unsaved draft costs no write. */
  #present = new Set<string>();
  #timer: ReturnType<typeof setTimeout> | undefined;
  #chain: Promise<unknown> = Promise.resolve();
  #closed = false;
  constructor(
    readonly store: DraftStore,
    readonly page: string,
    readonly failed: (failed: boolean) => void,
  ) {}
  async load() {
    const saved = await this.store.load(this.page);
    for (const target of saved?.keys() ?? []) this.#present.add(target);
    return saved;
  }
  set(target: string, edit: ComposerEdit | null) {
    if (this.#closed) return;
    if (edit !== null && persistable(edit)) {
      this.#present.add(target);
      this.#pending.set(target, structuredClone(edit));
    } else if (this.#present.delete(target)) this.#pending.set(target, null);
    else return;
    clearTimeout(this.#timer);
    this.#timer = setTimeout(() => void this.flush(), WRITE_DELAY_MS);
  }
  flush(): Promise<void> {
    clearTimeout(this.#timer);
    this.#chain = this.#chain.then(async () => {
      if (!this.#pending.size) return;
      const batch = this.#pending;
      this.#pending = new Map();
      const durable = await this.store.apply(this.page, batch);
      if (!durable) {
        // Anything typed since the failed attempt is newer than the batch.
        for (const [target, edit] of batch)
          if (!this.#pending.has(target)) this.#pending.set(target, edit);
      }
      this.failed(!durable);
    });
    return this.#chain as Promise<void>;
  }
  /** Stops accepting changes after a final best-effort write. */
  close(): Promise<void> {
    const last = this.flush();
    this.#closed = true;
    return last;
  }
}
