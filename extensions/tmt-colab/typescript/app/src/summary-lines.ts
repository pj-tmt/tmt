/** The two-line summary an author writes on a structured page value (`shell`, `work`):
 * `{line1,line2,attribution}`. The admitted writer supplies attribution; an action body never
 * does. Mirrors `summary_lines.rs`. */
import { coreIdentity, id, keys, nonempty, plain, requireValue } from './closed.js';

export const SUMMARY_LINE1_POINTS = 24;
export const SUMMARY_LINE2_POINTS = 60;
export interface Attribution {
  writerId: string;
  timeMs: number;
  /** Core-resolved display metadata, never proof of agent authority. */
  displayAgent?: { name: string; id?: string };
}
export interface SummaryLines {
  line1: string;
  line2: string;
  attribution: Attribution;
}
/** The two lines of an action body or a stored summary, each within its code-point limit. */
export function validateLines(value: Record<string, unknown>) {
  for (const [key, count] of [
    ['line1', SUMMARY_LINE1_POINTS],
    ['line2', SUMMARY_LINE2_POINTS],
  ] as const) {
    const line = value[key];
    plain(line, count * 4);
    requireValue([...line].length <= count);
  }
}
export function validateAttribution(value: unknown): asserts value is Attribution {
  const author = keys(value, ['writerId', 'timeMs'], ['displayAgent']);
  id(author.writerId);
  requireValue(Number.isSafeInteger(author.timeMs) && (author.timeMs as number) >= 0);
  if (Object.hasOwn(author, 'displayAgent')) {
    const agent = keys(author.displayAgent, ['name'], ['id']);
    nonempty(agent.name, 128);
    if (Object.hasOwn(agent, 'id')) coreIdentity(agent.id);
  }
}
/** A stored summary: both lines and the system attribution. */
export function validateSummary(value: unknown): asserts value is SummaryLines {
  const summary = keys(value, ['line1', 'line2', 'attribution']);
  validateLines(summary);
  validateAttribution(summary.attribution);
}
/** The summary an admitted `summary.set` action stores: its two lines plus the write owner's
 * attribution. */
export function setSummary(input: unknown, attribution: Attribution): SummaryLines {
  const body = keys(input, ['line1', 'line2']);
  validateLines(body);
  validateAttribution(attribution);
  return { line1: body.line1 as string, line2: body.line2 as string, attribution };
}
