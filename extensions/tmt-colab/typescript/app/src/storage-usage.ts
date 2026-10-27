import { exactKeys, requireValue } from '@tmt/colab-client';
import { jsonResponse } from './registration.js';

export interface StorageRow {
  pageId: string;
  title: string | null;
  contentBytes: number;
  historyBytes: number;
  commentsBytes: number;
  attachmentsBytes: number | null;
  unusedAttachmentBytes: number | null;
  totalBytes: number | null;
  freeable: { history: number; files: number };
  lastCleanedAutomaticallyAtMs: number | null;
  skipped: string | null;
}
export interface ReclaimResult {
  version: 1;
  page: string;
  dryRun: boolean;
  freeable: { history: number; files: number };
  freed: { history: number | null; files: number };
  kept: string[];
  skipped: string | null;
  error?: { code: string; message: string };
}
export interface StoragePort {
  read(page: string, signal?: AbortSignal): Promise<StorageRow>;
  reclaim(page: string, signal?: AbortSignal): Promise<ReclaimResult>;
}
function bytes(value: unknown): value is number {
  return Number.isSafeInteger(value) && (value as number) >= 0;
}
function nullableBytes(value: unknown): boolean {
  return value === null || bytes(value);
}
function quantities(
  value: unknown,
  nullable: boolean,
): asserts value is { history: number | null; files: number } {
  exactKeys(value, ['history', 'files']);
  requireValue(
    (nullable ? nullableBytes(value.history) : bytes(value.history)) && bytes(value.files),
  );
}
function row(value: unknown, page: string): StorageRow {
  exactKeys(value, [
    'pageId',
    'title',
    'contentBytes',
    'historyBytes',
    'commentsBytes',
    'attachmentsBytes',
    'unusedAttachmentBytes',
    'totalBytes',
    'freeable',
    'lastCleanedAutomaticallyAtMs',
    'skipped',
  ]);
  requireValue(value.pageId === page && (value.title === null || typeof value.title === 'string'));
  requireValue(
    bytes(value.contentBytes) && bytes(value.historyBytes) && bytes(value.commentsBytes),
  );
  requireValue(
    nullableBytes(value.attachmentsBytes) &&
      nullableBytes(value.unusedAttachmentBytes) &&
      nullableBytes(value.totalBytes) &&
      nullableBytes(value.lastCleanedAutomaticallyAtMs),
  );
  requireValue(value.skipped === null || typeof value.skipped === 'string');
  quantities(value.freeable, false);
  requireValue((value.freeable.history as number) <= (value.historyBytes as number));
  return value as unknown as StorageRow;
}
function result(value: unknown, page: string, ok: boolean): ReclaimResult {
  exactKeys(value, [
    'version',
    'page',
    'dryRun',
    'freeable',
    'freed',
    'kept',
    'skipped',
    ...(!ok ? ['error'] : []),
  ]);
  requireValue(value.version === 1 && value.page === page && value.dryRun === false);
  quantities(value.freeable, false);
  quantities(value.freed, true);
  if (ok) requireValue(value.freed.history !== null);
  requireValue(
    Array.isArray(value.kept) && value.kept.every((v: unknown) => typeof v === 'string'),
  );
  requireValue(value.skipped === null || typeof value.skipped === 'string');
  if (!ok) {
    exactKeys(value.error, ['code', 'message']);
    requireValue(
      value.error.code === 'COLAB_UNAVAILABLE' && typeof value.error.message === 'string',
    );
  }
  return value as unknown as ReclaimResult;
}
export function storagePort(mount: URL): StoragePort {
  async function post(path: string, page: string, signal?: AbortSignal) {
    const response = await fetch(new URL(path, mount), {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ version: 1, page, dryRun: false }),
      signal: signal
        ? AbortSignal.any([signal, AbortSignal.timeout(10000)])
        : AbortSignal.timeout(10000),
    });
    if (!response.ok && !(path === 'api/storage/reclaim' && response.status === 503)) {
      await jsonResponse(response, 8192);
    }
    const value = await jsonResponse(new Response(response.body, { status: 200 }), 8192);
    return { response, value };
  }
  return {
    async read(page, signal) {
      const { response, value } = await post('api/storage', page, signal);
      requireValue(response.ok);
      return row(value, page);
    },
    async reclaim(page, signal) {
      const { response, value } = await post('api/storage/reclaim', page, signal);
      requireValue(response.ok || response.status === 503);
      return result(value, page, response.ok);
    },
  };
}
