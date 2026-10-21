/** Closed shell values and detached actions; no target access or publication authority. */
import { byteLength, canonical, id, keys, object, plain, requireValue } from './closed.js';
import {
  setSummary,
  validateSummary,
  type Attribution,
  type SummaryLines,
} from './summary-lines.js';

export type ShellItem = (
  | { kind: 'page'; pageId: string }
  | { kind: 'anchor'; pageId: string; anchor: string }
  | { kind: 'reading'; pageId: string; readingId: string }
) & { note?: string; audience?: string };
export interface Shell {
  version: 1;
  items: Record<string, ShellItem>;
  order: string[];
  summary?: SummaryLines;
}
export const SHELL_ITEMS = 256;
export const SHELL_BYTES = 128 * 1024;
export function validShellAnchor(value: string): boolean {
  return /^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$/.test(value);
}
export function validateShellItem(value: unknown): asserts value is ShellItem {
  const item = object(value);
  const required =
    item.kind === 'page'
      ? ['kind', 'pageId']
      : item.kind === 'anchor'
        ? ['kind', 'pageId', 'anchor']
        : item.kind === 'reading'
          ? ['kind', 'pageId', 'readingId']
          : undefined;
  requireValue(required);
  keys(item, required, ['note', 'audience']);
  id(item.pageId);
  if (item.kind === 'anchor')
    requireValue(typeof item.anchor === 'string' && validShellAnchor(item.anchor));
  if (item.kind === 'reading') id(item.readingId);
  for (const [key, bytes] of [
    ['note', 512],
    ['audience', 128],
  ] as const)
    if (Object.hasOwn(item, key)) plain(item[key], bytes);
}
export function validateShell(value: unknown): asserts value is Shell {
  const shell = keys(value, ['version', 'items', 'order'], ['summary']);
  requireValue(shell.version === 1);
  const items = object(shell.items);
  requireValue(Object.keys(items).length <= SHELL_ITEMS);
  for (const [key, item] of Object.entries(items)) {
    id(key);
    validateShellItem(item);
  }
  requireValue(Array.isArray(shell.order) && shell.order.length === Object.keys(items).length);
  const seen = new Set<string>();
  for (const entry of shell.order) {
    id(entry);
    requireValue(Object.hasOwn(items, entry) && !seen.has(entry));
    seen.add(entry);
  }
  if (Object.hasOwn(shell, 'summary')) validateSummary(shell.summary);
  requireValue(byteLength(canonical(shell)) <= SHELL_BYTES);
}
/** The write owner supplies attribution; action JSON cannot claim an author or a time. */
export function applyShellAction(
  base: unknown,
  action: string,
  input: unknown,
  attribution: Attribution,
): Shell {
  validateShell(base);
  const next = structuredClone(base);
  const body = object(input);
  if (action === 'summary.set') {
    next.summary = setSummary(body, attribution);
  } else if (action === 'summary.clear') {
    keys(body, []);
    delete next.summary;
  } else {
    const spec: Record<string, [string[], string[]]> = {
      'item.add': [['itemId', 'item'], ['beforeId']],
      'item.remove': [['itemId'], []],
      'item.move': [['itemId'], ['beforeId']],
      'item.set': [['itemId', 'item'], []],
      'reading.suggest': [
        ['itemId', 'pageId', 'readingId'],
        ['beforeId', 'note', 'audience'],
      ],
    };
    requireValue(Object.hasOwn(spec, action));
    keys(body, ...spec[action]);
    id(body.itemId);
    const key = body.itemId;
    const position = () => {
      if (!Object.hasOwn(body, 'beforeId')) return next.order.length;
      id(body.beforeId);
      const index = next.order.indexOf(body.beforeId);
      requireValue(body.beforeId !== key && index >= 0);
      return index;
    };
    if (action === 'item.add' || action === 'reading.suggest') {
      requireValue(!Object.hasOwn(next.items, key));
      const item =
        action === 'item.add'
          ? body.item
          : {
              kind: 'reading',
              pageId: body.pageId,
              readingId: body.readingId,
              ...(Object.hasOwn(body, 'note') ? { note: body.note } : {}),
              ...(Object.hasOwn(body, 'audience') ? { audience: body.audience } : {}),
            };
      validateShellItem(item);
      const index = position();
      next.items[key] = structuredClone(item);
      next.order.splice(index, 0, key);
    } else {
      requireValue(Object.hasOwn(next.items, key));
      if (action === 'item.set') {
        validateShellItem(body.item);
        next.items[key] = structuredClone(body.item);
      } else {
        next.order = next.order.filter((id) => id !== key);
        if (action === 'item.remove') delete next.items[key];
        else next.order.splice(position(), 0, key);
      }
    }
  }
  validateShell(next);
  return next;
}
