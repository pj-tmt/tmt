import { editRequest, encodeBinary, text as bytesOf } from '@tmt/colab-client';
import type { OwnRecord } from './fold-protocol.js';
import { sealedBound } from './objects.js';
import { EditTooLarge } from './save.js';
import { hunksOf } from './save-hunks.js';

/** An edit request is one log document: its sealed envelope fits the canonical fields one hosted
 * document holds (`contracts/remote-channel-v1.md`), so a request is never split across
 * documents. The same bound holds for the local door, which has no document of its own. */
export const REQUEST_DOCUMENT_BYTES = 393_216;
/** The bound of the sealed envelope that carries an update of `update` bytes, or throws when a
 * request that large would not fit one document. */
export function requestFitsDocument(update: number) {
  const sealed = sealedBound(update);
  if (sealed > REQUEST_DOCUMENT_BYTES) throw new EditTooLarge(sealed, REQUEST_DOCUMENT_BYTES);
}

/** The anchored hunks when they fit the request's bounds; otherwise the whole source travels
 * alone, as a Save does when its hunks do not fit a frame. */
function carriedHunks(base: string, source: string) {
  const hunks = hunksOf(base, source);
  if (!hunks || hunks.length > editRequest.HUNKS) return {};
  const bytes = hunks.reduce((n, h) => n + bytesOf(h.old).length + bytesOf(h.new).length, 0);
  return bytes <= editRequest.HUNK_BYTES ? { hunks } : {};
}
/** The `edit-request` record asking the page's owner to save `source`, an edit of `base`. A
 * source over what one request carries is refused here, before anything is sealed or sent. */
export async function buildEditRequest(
  base: string,
  source: string,
): Promise<editRequest.EditRequest> {
  const size = bytesOf(source).length;
  if (size > editRequest.SOURCE_BYTES) throw new EditTooLarge(size, editRequest.SOURCE_BYTES);
  const operationId = crypto.randomUUID();
  return editRequest.editRequest(
    {
      version: 1,
      kind: 'edit-request',
      operationId,
      baseSha256: encodeBinary(await editRequest.sourceDigest(base)),
      sourceSha256: encodeBinary(await editRequest.sourceDigest(source)),
      source,
      ...carriedHunks(base, source),
    },
    'intents',
    operationId,
  );
}
/** The one own-stream record that carries a request, keyed by its operation ID. */
export const requestRecord = (request: editRequest.EditRequest): OwnRecord => ({
  root: 'intents',
  key: request.operationId,
  value: request as unknown as OwnRecord['value'],
});

/** The one thing a sender of requests has in return for them: where it sends, as the sender's
 * own writer does (`Writer.submitOwnRecords`). */
export interface RequestTransport {
  submitOwnRecords(records: OwnRecord[], fits: (update: number) => void): Promise<void>;
}
/** Sends one edit request. A request that cannot be sent as one document is refused before the
 * transport is touched, and again once its update is prepared and before it is sealed. */
export async function sendEditRequest(transport: RequestTransport, base: string, source: string) {
  const request = await buildEditRequest(base, source);
  await transport.submitOwnRecords([requestRecord(request)], requestFitsDocument);
  return request;
}

/** How long a sent edit waits to see the page change before it says it is still waiting. */
export const EDIT_WAITING_MS = 10_000;
/** What became of a request the owner's Colab was asked to apply. A link gets no reply to it: the
 * page is the only answer. `applied` when the page shows exactly the proposed text, `changed`
 * when it shows anything else (another edit, or a merge), so part of the request may be missing. */
export interface EditOutcome {
  kind: 'applied' | 'changed';
  source: string;
}
/** The page text a link last saw, and the edits waiting for it to change. */
export class PageAnswer {
  #shown = '';
  #ended = false;
  #waiters = new Set<() => void>();
  get shown() {
    return this.#shown;
  }
  show(source: string) {
    this.#shown = source;
    this.#wake();
  }
  /** The session is over; every edit still waiting is refused. */
  end() {
    this.#ended = true;
    this.#wake();
  }
  /** Waits for the page to leave `before`, the text it showed when the request was sent. Calls
   * `onWaiting` once if it has not after `EDIT_WAITING_MS`; the wait itself has no end but the
   * page's answer or the session's. */
  async answer(proposed: string, before: string, onWaiting: () => void): Promise<EditOutcome> {
    const timer = setTimeout(onWaiting, EDIT_WAITING_MS);
    try {
      for (;;) {
        if (this.#shown === proposed) return { kind: 'applied', source: proposed };
        if (this.#shown !== before) return { kind: 'changed', source: this.#shown };
        if (this.#ended) throw new Error('Page closed');
        await new Promise<void>((resolve) => this.#waiters.add(resolve));
      }
    } finally {
      clearTimeout(timer);
    }
  }
  #wake() {
    const waiting = [...this.#waiters];
    this.#waiters.clear();
    for (const resolve of waiting) resolve();
  }
}
