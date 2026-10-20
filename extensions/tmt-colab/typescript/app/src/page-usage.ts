import { exactKeys, requireValue } from '@tmt/colab-client';

/** Which of the page's three limits `percent` is: combining changes makes room only for the
 * first two. Mirrors `page::Limiting`. */
export type Limiting = 'changes' | 'recent-data' | 'stored-size';
const LIMITING: readonly string[] = ['changes', 'recent-data', 'stored-size'];

/** How full a page is, as the serve reports it (`page::PageUsage`). The browser shows these
 * numbers and never recomputes them. */
export interface PageUsage {
  updates: number;
  updateLimit: number;
  tailBytes: number;
  tailBytesLimit: number;
  checkpointBytes: number;
  otherDeviceUpdates: number;
  stateBytes: number;
  /** Absent for a state under 256 KiB, where `stateBytes` bounds it. */
  gzipBytes?: number;
  gzipLimit: number;
  percent: number;
  limiting: Limiting;
}
export type Skipped = 'nothing-to-combine' | 'not-combined';
export type LeftWhy = 'not-owner-device' | 'published' | 'too-large' | 'refused';
const LEFT_WHY: readonly string[] = ['not-owner-device', 'published', 'too-large', 'refused'];
/** Another device's stream that a compaction combined into an owner fold. */
export interface FoldedStream {
  stream: string;
  through: number;
  mergedObjects: number;
}
/** A stream with changes that a compaction left as it is, and why. */
export interface LeftStream {
  stream: string;
  why: LeftWhy;
}
/** What one requested compaction did (`page::compact::Report`). */
export interface CompactReport {
  pageId: string;
  compacted: boolean;
  skipped?: Skipped;
  by: string;
  through: number;
  mergedObjects: number;
  folded?: FoldedStream[];
  left?: LeftStream[];
  unfolded?: number;
  sourceSha256: string;
  before: PageUsage;
  after: PageUsage;
}
/** The serve refused the request; the code is the stable `COLAB_*` code. */
export class UsageRefused extends Error {
  constructor(
    readonly code: string,
    message: string,
  ) {
    super(message);
  }
}

function count(value: unknown): number {
  requireValue(typeof value === 'number' && Number.isSafeInteger(value) && value >= 0);
  return value as number;
}
export function pageUsage(value: unknown): PageUsage {
  requireValue(typeof value === 'object' && value !== null && !Array.isArray(value));
  const raw = value as Record<string, unknown>;
  const gzip = Object.hasOwn(raw, 'gzipBytes');
  exactKeys(raw, [
    'updates',
    'updateLimit',
    'tailBytes',
    'tailBytesLimit',
    'checkpointBytes',
    'otherDeviceUpdates',
    'stateBytes',
    ...(gzip ? ['gzipBytes'] : []),
    'gzipLimit',
    'percent',
    'limiting',
  ]);
  requireValue(typeof raw.limiting === 'string' && LIMITING.includes(raw.limiting));
  const usage: PageUsage = {
    updates: count(raw.updates),
    updateLimit: count(raw.updateLimit),
    tailBytes: count(raw.tailBytes),
    tailBytesLimit: count(raw.tailBytesLimit),
    checkpointBytes: count(raw.checkpointBytes),
    otherDeviceUpdates: count(raw.otherDeviceUpdates),
    stateBytes: count(raw.stateBytes),
    ...(gzip ? { gzipBytes: count(raw.gzipBytes) } : {}),
    gzipLimit: count(raw.gzipLimit),
    percent: count(raw.percent),
    limiting: raw.limiting as Limiting,
  };
  // The limits are the serve's own and never zero: a zero would make every share meaningless.
  requireValue(usage.updateLimit > 0 && usage.tailBytesLimit > 0 && usage.gzipLimit > 0);
  requireValue(usage.otherDeviceUpdates <= usage.updates);
  return usage;
}
function streamList<T>(value: unknown, keys: string[], read: (row: Record<string, unknown>) => T) {
  requireValue(Array.isArray(value) && value.length > 0 && value.length <= 256);
  return value.map((row) => {
    requireValue(typeof row === 'object' && row !== null && !Array.isArray(row));
    exactKeys(row, keys);
    const record = row as Record<string, unknown>;
    requireValue(typeof record.stream === 'string' && record.stream.length <= 64);
    return read(record);
  });
}
export function compactReport(value: unknown): CompactReport {
  requireValue(typeof value === 'object' && value !== null && !Array.isArray(value));
  const raw = value as Record<string, unknown>;
  const skipped = Object.hasOwn(raw, 'skipped');
  const [folded, left, unfolded] = ['folded', 'left', 'unfolded'].map((key) =>
    Object.hasOwn(raw, key),
  );
  exactKeys(raw, [
    'pageId',
    'compacted',
    ...(skipped ? ['skipped'] : []),
    'by',
    'through',
    'mergedObjects',
    ...(folded ? ['folded'] : []),
    ...(left ? ['left'] : []),
    ...(unfolded ? ['unfolded'] : []),
    'sourceSha256',
    'before',
    'after',
  ]);
  requireValue(typeof raw.pageId === 'string' && raw.pageId.length <= 64);
  requireValue(typeof raw.compacted === 'boolean');
  requireValue(typeof raw.by === 'string' && raw.by.length <= 64);
  requireValue(typeof raw.sourceSha256 === 'string' && /^[0-9a-f]{64}$/.test(raw.sourceSha256));
  if (skipped) requireValue(raw.skipped === 'nothing-to-combine' || raw.skipped === 'not-combined');
  // A report either combined something or says why not, never both and never neither.
  requireValue(raw.compacted === !skipped);
  return {
    pageId: raw.pageId as string,
    compacted: raw.compacted as boolean,
    ...(skipped ? { skipped: raw.skipped as Skipped } : {}),
    by: raw.by as string,
    through: count(raw.through),
    mergedObjects: count(raw.mergedObjects),
    ...(folded
      ? {
          folded: streamList(raw.folded, ['stream', 'through', 'mergedObjects'], (row) => ({
            stream: row.stream as string,
            through: count(row.through),
            mergedObjects: count(row.mergedObjects),
          })),
        }
      : {}),
    ...(left
      ? {
          left: streamList(raw.left, ['stream', 'why'], (row) => {
            requireValue(typeof row.why === 'string' && LEFT_WHY.includes(row.why));
            return { stream: row.stream as string, why: row.why as LeftWhy };
          }),
        }
      : {}),
    ...(unfolded ? { unfolded: count(raw.unfolded) } : {}),
    sourceSha256: raw.sourceSha256 as string,
    before: pageUsage(raw.before),
    after: pageUsage(raw.after),
  };
}
/** A `usageresult` or `compactresult` frame: the payload under `key`, or the serve's refusal. */
export function pageReply<T>(
  frame: Record<string, unknown>,
  key: 'usage' | 'report',
  parse: (value: unknown) => T,
): T {
  const failed = Object.hasOwn(frame, 'error');
  exactKeys(frame, ['version', 'type', 'space', 'page', 'epoch', failed ? 'error' : key]);
  if (!failed) return parse(frame[key]);
  const error = frame.error as Record<string, unknown> | null;
  requireValue(typeof error === 'object' && error !== null && !Array.isArray(error));
  exactKeys(error as Record<string, unknown>, ['code', 'message']);
  const { code, message } = error as { code: unknown; message: unknown };
  requireValue(typeof code === 'string' && /^[A-Z_]{1,64}$/.test(code));
  requireValue(typeof message === 'string' && message.length <= 2048);
  throw new UsageRefused(code as string, message as string);
}

/** Where a page stands against its limits: plain below 80%, nearly full from 80%, full at 100%. */
export type UsageLevel = 'ok' | 'nearly-full' | 'full';
export function usageLevel(usage: PageUsage): UsageLevel {
  return usage.percent >= 100 ? 'full' : usage.percent >= 80 ? 'nearly-full' : 'ok';
}
/** Compact is offered only when combining can make room: the tail is the limit, and this device
 * has changes of its own to combine. Otherwise the surface says why instead of offering a no-op. */
export function compactOffered(usage: PageUsage): boolean {
  return usage.limiting !== 'stored-size' && usage.updates > usage.otherDeviceUpdates;
}
