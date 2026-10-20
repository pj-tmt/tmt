import { expect, it } from 'vite-plus/test';
import { MAX_UNRELATED_WINDOW, unrelatedWindow } from '../src/admission.js';

/** The serve's rule for an append that cites an earlier membership revision (colab-v1, "Append at an
 * earlier membership revision"), mirrored by the owner browser so it drops only what the serve would
 * refuse. The same table runs against the serve in `tests/registration.rs`. */
const PAGE = '40000000-0000-4000-8000-000000000001';
const ELSEWHERE = '40000000-0000-4000-8000-000000000153';
const SOMEONE = '40000000-0000-4000-8000-000000000154';
const statement = (operation: string, value: Record<string, unknown>) =>
  ({ payload: { operation, value } }) as never;
const log = (...statements: unknown[]) => statements as never[];
const window = (statements: never[], cited: bigint) =>
  unrelatedWindow(statements, BigInt(statements.length), cited, PAGE);

const elsewhere = [
  statement('page.share', { pageId: ELSEWHERE, mode: 'private', epoch: '1' }),
  statement('page.share', { pageId: ELSEWHERE, mode: 'link', epoch: '1' }),
  statement('epoch.advance', { pageId: ELSEWHERE, epoch: '2' }),
  statement('page.history', { pageId: ELSEWHERE, mode: 'current' }),
  statement('retention.set', { pageId: ELSEWHERE, days: 30 }),
  statement('page.archive', { pageId: ELSEWHERE }),
  statement('page.delete', { pageId: ELSEWHERE }),
];
const related = [
  statement('page.share', { pageId: PAGE, mode: 'link', epoch: '1' }),
  statement('epoch.advance', { pageId: PAGE, epoch: '2' }),
  statement('page.history', { pageId: PAGE, mode: 'current' }),
  statement('retention.set', { pageId: PAGE, days: 30 }),
  statement('page.archive', { pageId: PAGE }),
  statement('page.delete', { pageId: PAGE }),
  statement('member.role', { memberId: SOMEONE, role: 'viewer', cuts: [] }),
  statement('member.remove', { memberId: SOMEONE, cuts: [] }),
  // Naming another page is not page scope: only a statement that is scoped to a page can be unrelated.
  statement('member.add', { memberId: SOMEONE, pages: [ELSEWHERE] }),
  statement('link.add', { linkId: SOMEONE, pages: [ELSEWHERE] }),
  // A kind this rule has not decided about is related until it is listed.
  statement('page.future', { pageId: ELSEWHERE }),
  statement('link.remove', { linkId: SOMEONE, cuts: [] }),
  statement('device.revoke', { deviceId: SOMEONE, cuts: [] }),
  statement('bridge.add', { machineId: SOMEONE }),
];

it('stands when every statement it missed is scoped to another page', () => {
  const all = log(statement('page.history', { pageId: PAGE, mode: 'shared' }), ...elsewhere);
  // Revision 1 is the history of this page: an append citing 1 or later missed only other pages.
  expect(window(all, 1n)).toBe(true);
  expect(window(all, 2n)).toBe(true);
  expect(window(all, 0n)).toBe(false);
  // Citing the head misses nothing.
  expect(window(all, BigInt(all.length))).toBe(true);
});

for (const [index, statements] of related.entries())
  it(`does not stand past a statement that can touch this page or a writer (${index})`, () => {
    const all = log(...elsewhere, statements, ...elsewhere);
    expect(window(all, 0n)).toBe(false);
    // Unrelated statements before it do not hide it, and ones after it do not either.
    expect(window(all, BigInt(elsewhere.length))).toBe(false);
    expect(window(all, BigInt(elsewhere.length) + 1n)).toBe(true);
  });

it('is bounded, never answers for a revision the log has not reached, and needs the whole window', () => {
  const run = log(...Array.from({ length: Number(MAX_UNRELATED_WINDOW) + 1 }, () => elsewhere[0]));
  expect(window(run, 1n)).toBe(true);
  expect(window(run, 0n)).toBe(false);
  expect(unrelatedWindow(run, BigInt(run.length), BigInt(run.length) + 1n, PAGE)).toBe(false);
  expect(unrelatedWindow(run, undefined, 0n, PAGE)).toBe(false);
  // A log shorter than the head leaves part of the window unseen.
  expect(unrelatedWindow(log(elsewhere[0]), 3n, 1n, PAGE)).toBe(false);
});
