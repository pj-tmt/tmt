import { afterEach, expect, it, vi } from 'vite-plus/test';
import { editRequest } from '@tmt/colab-client';
import type { OwnRecord } from '../src/fold-protocol.js';
import {
  EDIT_WAITING_MS,
  PageAnswer,
  REQUEST_DOCUMENT_BYTES,
  buildEditRequest,
  requestFitsDocument,
  sendEditRequest,
  type RequestTransport,
} from '../src/link-edit.js';
import { UPDATE_ENVELOPE_BYTES, sealedBound } from '../src/objects.js';
import { EditTooLarge, saveMessage } from '../src/save.js';
import { text } from '../src/strings.js';

afterEach(() => vi.useRealTimers());

const BASE = 'line one\nline two\nline three\nline four\nline five\n';
const EDITED = 'line one\nline TWO\nline three\nline four\nline five\n';

/** The transport that records what it was asked to send. `fits` is what the real writer calls
 * with the size of the update it prepared. */
function recording(prepared = 1024) {
  const sent: OwnRecord[][] = [];
  const transport: RequestTransport = {
    async submitOwnRecords(records, fits) {
      fits(prepared);
      sent.push(records);
    },
  };
  return { sent, transport };
}

it('builds a request that the shared validator accepts and that names both texts', async () => {
  const request = await buildEditRequest(BASE, EDITED);
  const checked = await editRequest.verifiedEditRequest(request, 'intents', request.operationId);
  expect(checked).toEqual(request);
  expect(request.source).toBe(EDITED);
  expect(request.baseSha256).not.toBe(request.sourceSha256);
  expect(request.hunks).toEqual([{ oldStart: 1, old: BASE, new: EDITED }]);
  // A different base is a different digest; the same text twice is the same.
  expect((await buildEditRequest(EDITED, EDITED)).baseSha256).toBe(request.sourceSha256);
  expect((await buildEditRequest(BASE, EDITED)).operationId).not.toBe(request.operationId);
});

it('sends the whole source alone when there is no base to anchor to', async () => {
  const request = await buildEditRequest('', 'first text');
  expect(request).not.toHaveProperty('hunks');
  await editRequest.verifiedEditRequest(request, 'intents', request.operationId);
});

it('carries a source of exactly the limit, and refuses one byte over, counting UTF-8 bytes', async () => {
  const exact = 'a'.repeat(editRequest.SOURCE_BYTES);
  await editRequest.verifiedEditRequest(await buildEditRequest('', exact));
  const over = await buildEditRequest('', `${exact}a`).catch((error) => error);
  expect(over).toBeInstanceOf(EditTooLarge);
  expect(over).toMatchObject({
    size: editRequest.SOURCE_BYTES + 1,
    limit: editRequest.SOURCE_BYTES,
  });
  // Two-byte characters: half the limit in characters is the limit in bytes, plus one is over.
  const wide = 'é'.repeat(editRequest.SOURCE_BYTES / 2);
  await editRequest.verifiedEditRequest(await buildEditRequest('', wide));
  await expect(buildEditRequest('', `${wide}a`)).rejects.toBeInstanceOf(EditTooLarge);
});

it('drops hunks that are over the request bounds instead of refusing the edit', async () => {
  // 40 000 two-byte characters changed on one line: 40 000 characters, 80 000 bytes each side.
  const old = `${'é'.repeat(40_000)}\n`;
  const changed = `${'è'.repeat(40_000)}\n`;
  const request = await buildEditRequest(old, changed);
  expect(request).not.toHaveProperty('hunks');
  expect(request.source).toBe(changed);
  await editRequest.verifiedEditRequest(request, 'intents', request.operationId);
});

it('refuses an oversized edit before the transport is asked to send anything', async () => {
  const { sent, transport } = recording();
  const tooBig = 'a'.repeat(editRequest.SOURCE_BYTES + 1);
  await expect(sendEditRequest(transport, BASE, tooBig)).rejects.toBeInstanceOf(EditTooLarge);
  expect(sent).toEqual([]);
});

it('sends one record, keyed by the operation, for an edit that fits', async () => {
  const { sent, transport } = recording();
  const request = await sendEditRequest(transport, BASE, EDITED);
  expect(sent).toHaveLength(1);
  expect(sent[0]).toEqual([{ root: 'intents', key: request.operationId, value: request }]);
});

it('refuses a request whose prepared update would not fit one document, before it is sent', async () => {
  const { sent, transport } = recording(300_000);
  const error = await sendEditRequest(transport, BASE, EDITED).catch((e) => e);
  expect(error).toBeInstanceOf(EditTooLarge);
  expect(error).toMatchObject({ size: sealedBound(300_000), limit: REQUEST_DOCUMENT_BYTES });
  expect(sent).toEqual([]);
});

it('keeps the sealed update bound of every writer inside the one-document bound', () => {
  expect(UPDATE_ENVELOPE_BYTES).toBeLessThanOrEqual(REQUEST_DOCUMENT_BYTES);
  expect(REQUEST_DOCUMENT_BYTES).toBe(393_216);
  expect(() => requestFitsDocument(256 * 1024)).not.toThrow();
  expect(() => requestFitsDocument(295_000)).toThrow(EditTooLarge);
});

it('says the sizes of an edit and of a link in the too-large notice', () => {
  expect(saveMessage(new EditTooLarge(200 * 1024, 128 * 1024))).toBe(
    text.linkEditTooLarge(200 * 1024, 128 * 1024),
  );
  expect(text.linkEditTooLarge(200 * 1024, 128 * 1024)).toBe(
    'The edit is 200 KiB (204,800 bytes), over the 128 KiB (131,072 bytes) a link can send. Shorten it and save again.',
  );
});

it('answers applied when the page shows the proposed text, changed when it shows other text', async () => {
  const page = new PageAnswer();
  page.show('before');
  const applied = page.answer('proposed', 'before', vi.fn());
  page.show('proposed');
  await expect(applied).resolves.toEqual({ kind: 'applied', source: 'proposed' });
  // A merge, another editor, or a different digest: anything but the proposed text and the
  // unchanged page reads as changed, and never as applied.
  page.show('newer');
  const merged = page.answer('newer and mine', 'newer', vi.fn());
  page.show('newer and mine and more');
  await expect(merged).resolves.toEqual({ kind: 'changed', source: 'newer and mine and more' });
  // A page that already moved before the answer is asked for is changed too.
  await expect(page.answer('x', 'older', vi.fn())).resolves.toEqual({
    kind: 'changed',
    source: 'newer and mine and more',
  });
});

it('says it is still waiting once, after ten seconds, while the page is as it was', async () => {
  vi.useFakeTimers();
  const page = new PageAnswer();
  page.show('before');
  const waiting = vi.fn();
  let answer: unknown;
  void page.answer('proposed', 'before', waiting).then((value) => (answer = value));
  await vi.advanceTimersByTimeAsync(EDIT_WAITING_MS - 1);
  expect(waiting).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(1);
  expect(waiting).toHaveBeenCalledTimes(1);
  // Showing the same text again is not an answer, and nothing is said twice.
  page.show('before');
  await vi.advanceTimersByTimeAsync(60_000);
  expect(waiting).toHaveBeenCalledTimes(1);
  expect(answer).toBeUndefined();
  page.show('proposed');
  await vi.advanceTimersByTimeAsync(0);
  expect(answer).toEqual({ kind: 'applied', source: 'proposed' });
  // The answer cancelled the notice for good.
  await vi.advanceTimersByTimeAsync(60_000);
  expect(waiting).toHaveBeenCalledTimes(1);
});

it('refuses every edit still waiting when the session ends', async () => {
  vi.useFakeTimers();
  const page = new PageAnswer();
  page.show('before');
  const waiting = vi.fn();
  const pending = page.answer('proposed', 'before', waiting);
  const settled = expect(pending).rejects.toThrow();
  page.end();
  await settled;
  await vi.advanceTimersByTimeAsync(EDIT_WAITING_MS * 2);
  expect(waiting).not.toHaveBeenCalled();
});
