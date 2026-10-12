import { expect, it } from 'vite-plus/test';
import { outcomeOf } from '../src/page-size.js';
import {
  compactOffered,
  compactReport,
  pageReply,
  pageUsage,
  UsageRefused,
  usageLevel,
  type CompactReport,
  type PageUsage,
} from '../src/page-usage.js';

const usage = (extra: Partial<PageUsage> = {}): PageUsage => ({
  updates: 124,
  updateLimit: 200,
  tailBytes: 1_258_291,
  tailBytesLimit: 4_194_304,
  checkpointBytes: 4000,
  otherDeviceUpdates: 4,
  stateBytes: 1_262_291,
  gzipLimit: 5_000_000,
  percent: 62,
  limiting: 'changes',
  ...extra,
});
const report = (extra: Partial<CompactReport> = {}): CompactReport => ({
  pageId: '10000000-0000-4000-8000-000000000001',
  compacted: true,
  by: '30000000-0000-4000-8000-000000000001',
  through: 120,
  mergedObjects: 120,
  sourceSha256: 'a'.repeat(64),
  before: usage(),
  after: usage({ updates: 5, percent: 3 }),
  ...extra,
});
const frame = (extra: Record<string, unknown>) => ({
  version: 1,
  type: 'usageresult',
  space: 's',
  page: 'p',
  epoch: '1',
  ...extra,
});

it('reads exactly the usage the serve sends, with the gzipped size only when it was measured', () => {
  expect(pageUsage(usage())).toEqual(usage());
  expect(pageUsage(usage({ gzipBytes: 2_000_000, limiting: 'stored-size' }))).toMatchObject({
    gzipBytes: 2_000_000,
    limiting: 'stored-size',
  });
  const wrong: unknown[] = [
    null,
    [],
    { ...usage(), extra: 1 },
    { ...usage(), limiting: 'speed' },
    { ...usage(), updates: -1 },
    { ...usage(), updates: 1.5 },
    { ...usage(), updates: '3' },
    { ...usage(), updateLimit: 0 },
    { ...usage(), gzipLimit: 0 },
    { ...usage(), otherDeviceUpdates: 125 },
    { ...usage(), gzipBytes: undefined, percent: undefined },
  ];
  for (const value of wrong) expect(() => pageUsage(value), JSON.stringify(value)).toThrow();
  const { percent: _percent, ...missing } = usage();
  expect(() => pageUsage(missing)).toThrow();
});

it('reads a compaction report that either combined something or says why not', () => {
  expect(compactReport(report())).toEqual(report());
  const nothing = report({ compacted: false, skipped: 'nothing-to-combine', mergedObjects: 0 });
  expect(compactReport(nothing)).toEqual(nothing);
  const wrong: unknown[] = [
    { ...report(), extra: 1 },
    { ...report(), compacted: false },
    { ...report(), skipped: 'nothing-to-combine' },
    { ...report({ compacted: false }), skipped: 'because' },
    { ...report(), sourceSha256: 'abc' },
    { ...report(), through: -1 },
    { ...report(), after: { ...usage(), extra: 1 } },
    { ...report(), compacted: 'yes' },
  ];
  for (const value of wrong) expect(() => compactReport(value), JSON.stringify(value)).toThrow();
});

it("settles a reply with the payload or the serve's refusal, and rejects any other shape", () => {
  expect(pageReply(frame({ usage: usage() }), 'usage', pageUsage)).toEqual(usage());
  const refused = () =>
    pageReply(frame({ error: { code: 'COLAB_DENIED', message: 'No.' } }), 'usage', pageUsage);
  expect(refused).toThrow(UsageRefused);
  expect(refused).toThrow('No.');
  try {
    refused();
  } catch (error) {
    expect(error).toMatchObject({ code: 'COLAB_DENIED' });
  }
  const malformed: Record<string, unknown>[] = [
    frame({}),
    frame({ usage: usage(), error: { code: 'COLAB_DENIED', message: 'x' } }),
    frame({ usage: usage(), extra: 1 }),
    frame({ error: { code: 'denied', message: 'x' } }),
    frame({ error: { code: 'COLAB_DENIED' } }),
    frame({ error: { code: 'COLAB_DENIED', message: 'x', extra: 1 } }),
    frame({ error: 'COLAB_DENIED' }),
    frame({ error: null }),
  ];
  for (const value of malformed) {
    const read = () => pageReply(value, 'usage', pageUsage);
    expect(read, JSON.stringify(value)).toThrow();
    // A malformed reply is a protocol fault, never mistaken for a refusal.
    try {
      read();
    } catch (error) {
      expect(error).not.toBeInstanceOf(UsageRefused);
    }
  }
});

it('places a page plain below 80, nearly full from 80 and full at 100', () => {
  expect(usageLevel(usage({ percent: 0 }))).toBe('ok');
  expect(usageLevel(usage({ percent: 79 }))).toBe('ok');
  expect(usageLevel(usage({ percent: 80 }))).toBe('nearly-full');
  expect(usageLevel(usage({ percent: 99 }))).toBe('nearly-full');
  expect(usageLevel(usage({ percent: 100 }))).toBe('full');
  expect(usageLevel(usage({ percent: 140 }))).toBe('full');
});

it('offers Compact only when the changes are the limit and this device has some to combine', () => {
  expect(compactOffered(usage())).toBe(true);
  expect(compactOffered(usage({ limiting: 'recent-data' }))).toBe(true);
  expect(compactOffered(usage({ limiting: 'stored-size' }))).toBe(false);
  expect(compactOffered(usage({ updates: 4, otherDeviceUpdates: 4 }))).toBe(false);
  expect(compactOffered(usage({ updates: 5, otherDeviceUpdates: 4 }))).toBe(true);
  expect(compactOffered(usage({ updates: 0, otherDeviceUpdates: 0 }))).toBe(false);
});

it('words each compaction result as the UX rulings do', () => {
  expect(outcomeOf(report())).toEqual({
    kind: 'combined',
    merged: 120,
    after: report().after,
    before: report().before,
  });
  // Still at 80 or more afterwards, or left limited only by stored size.
  expect(outcomeOf(report({ after: usage({ percent: 80 }) })).kind).toBe('still-high');
  expect(outcomeOf(report({ after: usage({ percent: 20, limiting: 'stored-size' }) })).kind).toBe(
    'still-high',
  );
  expect(outcomeOf(report({ after: usage({ percent: 79 }) })).kind).toBe('combined');
  const nothing = report({ compacted: false, skipped: 'nothing-to-combine', after: usage() });
  expect(outcomeOf(nothing)).toEqual({ kind: 'nothing', usage: usage() });
  expect(outcomeOf(report({ compacted: false, skipped: 'not-combined' }))).toEqual({
    kind: 'failed',
  });
});
