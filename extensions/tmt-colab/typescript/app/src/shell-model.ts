import {
  READING_DETAILS_PAGES,
  type LatestComment,
  type NeedDescriptor,
  type ReadingDetails,
} from './reading-details.js';
import type { ShellCell } from './shell-grid.js';

/** The shell page's computed look, from the list data and the owner chapter details alone: the
 * greeting and counts, the chapters in order, and the older pages that fold into a list. Pure: the
 * caller supplies the clock. Nothing here reads or changes shell data beyond what #3733 defined. */

const DAY_MS = 86_400_000;
/** A page edited within this long counts as in progress, unless it carries its own work stage. */
export const IN_PROGRESS_MS = 7 * DAY_MS;

export type TimeOfDay = 'morning' | 'afternoon' | 'evening';

/** The local time-of-day word: morning 05:00-11:59, afternoon to 17:59, evening otherwise. */
export function timeOfDay(now: number): TimeOfDay {
  const hour = new Date(now).getHours();
  return hour >= 5 && hour < 12 ? 'morning' : hour >= 12 && hour < 18 ? 'afternoon' : 'evening';
}

/** Local midnight at or before `now`. */
export function startOfDay(now: number): number {
  const day = new Date(now);
  day.setHours(0, 0, 0, 0);
  return day.getTime();
}

const SMALL = ['zero', 'one', 'two', 'three', 'four', 'five', 'six', 'seven', 'eight', 'nine'];
/** Numbers one to nine are spelled out in running text; larger ones stay digits. */
export function spellSmallNumber(count: number): string {
  return Number.isInteger(count) && count >= 1 && count <= 9 ? SMALL[count] : String(count);
}

export type Stage = 'waiting' | 'in-progress';

/** What a chapter shows on its right: the passage that most needs the owner, else the page's latest
 * comment, else a note on the item; `pending` while the details of a waiting page are not here. */
export type Passage =
  | { kind: 'need'; need: NeedDescriptor; more: number }
  | { kind: 'latest'; latest: LatestComment }
  | { kind: 'note'; note: string }
  | { kind: 'pending' }
  | { kind: 'none' };

export interface Chapter {
  /** 1-based position, shown as 01, 02, … */
  number: number;
  pageId: string;
  /** Every item of the shell that names this page, in the shell's order. */
  itemIds: string[];
  title: string;
  titleMissing: boolean;
  author?: string;
  /** The first of the page's projects, when it has any. */
  label?: string;
  lastUpdateAtMs: number | null;
  stage: Stage;
  needsYou: number;
  passage: Passage;
}

/** A page that folds into the list of older pages: large-type rows, no passage. */
export interface EarlierRow {
  pageId: string;
  title: string;
  titleMissing: boolean;
  author?: string;
  lastUpdateAtMs: number | null;
}

export interface ShellModel {
  timeOfDay: TimeOfDay;
  /** The greeting's number, the same as the count row's first number. */
  waiting: number;
  counts: { waiting: number; inProgress: number; updatedToday: number };
  chapters: Chapter[];
  earlier: EarlierRow[];
  /** References whose page the space list does not show; they stay listed, never counted. */
  unavailable: string[];
}

/** One page as the shell sees it: every item that names it, in the shell's order. */
export interface Group {
  pageId: string;
  cells: ShellCell[];
}

function groupByPage(cells: readonly ShellCell[]): Group[] {
  const groups = new Map<string, Group>();
  for (const cell of cells) {
    if (cell.unavailable || cell.archived) continue;
    const group = groups.get(cell.pageId);
    if (group) group.cells.push(cell);
    else groups.set(cell.pageId, { pageId: cell.pageId, cells: [cell] });
  }
  return [...groups.values()];
}

/** The one place that decides whether a page not waiting on the owner is in progress. A page's own
 * work stage (#4050) wins here once the list carries it; until then it is a recent edit. */
export function inProgress(group: { lastUpdateAtMs: number | null }, now: number): boolean {
  return group.lastUpdateAtMs !== null && now - group.lastUpdateAtMs <= IN_PROGRESS_MS;
}

/** The passage a chapter shows. An item that names a pending target (a reading or an anchor on its
 * ID) puts that target first; otherwise the first pending item by rank. */
export function passageFor(
  group: Group,
  details: ReadingDetails | undefined,
  waiting: boolean,
): Passage {
  const ready = details?.state === 'ready' ? details : undefined;
  if (ready && ready.needs.length > 0) {
    const named = new Set(
      group.cells.flatMap((cell) => [cell.readingId, cell.anchor].filter((x) => x !== undefined)),
    );
    const need = ready.needs.find((candidate) => named.has(candidate.id)) ?? ready.needs[0];
    return { kind: 'need', need, more: Math.max(0, ready.needs.length - 1) };
  }
  if (ready?.latest) return { kind: 'latest', latest: ready.latest };
  const note = group.cells.find((cell) => cell.note !== undefined)?.note;
  if (note !== undefined && !(ready === undefined && waiting)) return { kind: 'note', note };
  return waiting && ready === undefined ? { kind: 'pending' } : { kind: 'none' };
}

/** The shell page's model. `details` holds what the owner chapter details said of each page so far;
 * a page without an entry, or with one that is not ready, simply has no passage yet. */
export function shellModel(
  cells: readonly ShellCell[],
  details: ReadonlyMap<string, ReadingDetails>,
  now: number,
): ShellModel {
  const today = startOfDay(now);
  const unavailable = cells.filter((cell) => cell.unavailable).map((cell) => cell.pageId);
  const order = new Map(cells.map((cell, index) => [cell.id, index]));
  const entries = groupByPage(cells).map((group) => {
    const first = group.cells[0];
    const needsYou = Math.max(...group.cells.map((cell) => cell.needsYou));
    return { group, first, needsYou, lastUpdateAtMs: first.lastUpdateAtMs };
  });
  const position = (entry: (typeof entries)[number]) => order.get(entry.first.id) ?? 0;
  const byRecent = (a: (typeof entries)[number], b: (typeof entries)[number]) =>
    (b.lastUpdateAtMs ?? -1) - (a.lastUpdateAtMs ?? -1) || position(a) - position(b);

  const waiting = entries.filter((entry) => entry.needsYou > 0).sort(byRecent);
  const rest = entries.filter((entry) => entry.needsYou === 0).sort(byRecent);
  const active = rest.filter((entry) => inProgress(entry, now));
  const older = rest.filter((entry) => !inProgress(entry, now));

  const chapter = (entry: (typeof entries)[number], stage: Stage, index: number): Chapter => ({
    number: index + 1,
    pageId: entry.group.pageId,
    itemIds: entry.group.cells.map((cell) => cell.id),
    title: entry.first.title,
    titleMissing: entry.first.titleMissing,
    ...(entry.first.author === undefined ? {} : { author: entry.first.author }),
    ...(entry.first.label === undefined ? {} : { label: entry.first.label }),
    lastUpdateAtMs: entry.lastUpdateAtMs,
    stage,
    needsYou: entry.needsYou,
    passage: passageFor(entry.group, details.get(entry.group.pageId), stage === 'waiting'),
  });
  const chapters = [
    ...waiting.map((entry) => ({ entry, stage: 'waiting' as const })),
    ...active.map((entry) => ({ entry, stage: 'in-progress' as const })),
  ].map(({ entry, stage }, index) => chapter(entry, stage, index));

  // The count the page header shows for one page is the number a chapter carries, so Home, the
  // header and the greeting agree: the sum over the distinct referenced pages.
  const waitingCount = entries.reduce((sum, entry) => sum + entry.needsYou, 0);
  return {
    timeOfDay: timeOfDay(now),
    waiting: waitingCount,
    counts: {
      waiting: waitingCount,
      inProgress: active.length,
      updatedToday: entries.filter((entry) => (entry.lastUpdateAtMs ?? -1) >= today).length,
    },
    chapters,
    earlier: older.map((entry) => ({
      pageId: entry.group.pageId,
      title: entry.first.title,
      titleMissing: entry.first.titleMissing,
      ...(entry.first.author === undefined ? {} : { author: entry.first.author }),
      lastUpdateAtMs: entry.lastUpdateAtMs,
    })),
    unavailable,
  };
}

/** The pages to ask the owner details about: the first chapters, in chapter order, at most the
 * details queue's cap. */
export function detailPageIds(model: ShellModel): string[] {
  return model.chapters.slice(0, READING_DETAILS_PAGES).map((chapter) => chapter.pageId);
}
