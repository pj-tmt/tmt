/** Closed shell values and detached actions; no target access or publication authority. */
export type ShellItem = (
  | { kind: 'page'; pageId: string }
  | { kind: 'anchor'; pageId: string; anchor: string }
  | { kind: 'reading'; pageId: string; readingId: string }
) & { note?: string; audience?: string };
export interface ShellAttribution {
  writerId: string;
  timeMs: number;
  /** Core-resolved display metadata, never proof of agent authority. */
  displayAgent?: { name: string; id?: string };
}
export interface Shell {
  version: 1;
  items: Record<string, ShellItem>;
  order: string[];
  summary?: { line1: string; line2: string; attribution: ShellAttribution };
}
export const SHELL_ITEMS = 256;
export const SHELL_BYTES = 128 * 1024;
const utf8 = new TextEncoder();
const coreId = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const generatedId = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
function requireValue(ok: unknown): asserts ok {
  if (!ok) throw new Error('Invalid shell');
}
function object(value: unknown): Record<string, unknown> {
  requireValue(value !== null && typeof value === 'object' && !Array.isArray(value));
  return value as Record<string, unknown>;
}
function keys(value: unknown, required: string[], optional: string[] = []) {
  const map = object(value);
  requireValue(required.every((key) => Object.hasOwn(map, key)));
  requireValue(
    Object.entries(map).every(
      ([key, value]) => (required.includes(key) || optional.includes(key)) && value !== null,
    ),
  );
  return map;
}
function id(value: unknown): asserts value is string {
  requireValue(typeof value === 'string' && generatedId.test(value));
}
function plain(value: unknown, bytes: number): asserts value is string {
  requireValue(
    typeof value === 'string' &&
      utf8.encode(value).length <= bytes &&
      !/[\p{Cc}\p{Cs}]/u.test(value),
  );
}
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
function lines(value: Record<string, unknown>) {
  for (const [key, count] of [
    ['line1', 24],
    ['line2', 60],
  ] as const) {
    const line = value[key];
    plain(line, count * 4);
    requireValue([...line].length <= count);
  }
}
export function validateShellAttribution(value: unknown): asserts value is ShellAttribution {
  const author = keys(value, ['writerId', 'timeMs'], ['displayAgent']);
  id(author.writerId);
  requireValue(Number.isSafeInteger(author.timeMs) && (author.timeMs as number) >= 0);
  if (Object.hasOwn(author, 'displayAgent')) {
    const agent = keys(author.displayAgent, ['name'], ['id']);
    plain(agent.name, 128);
    requireValue(agent.name.length > 0);
    if (Object.hasOwn(agent, 'id'))
      requireValue(
        typeof agent.id === 'string' &&
          coreId.test(agent.id) &&
          agent.id !== '00000000-0000-0000-0000-000000000000',
      );
  }
}
/** Only closed ASCII field names and UUID keys occur in a shell; lexical sorting is shared. */
export function canonicalShell(value: unknown): string {
  const sorted = (value: unknown): unknown => {
    if (Array.isArray(value)) return value.map(sorted);
    if (value !== null && typeof value === 'object')
      return Object.fromEntries(
        Object.entries(value)
          .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
          .map(([key, value]) => [key, sorted(value)]),
      );
    return value;
  };
  return JSON.stringify(sorted(value));
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
  if (Object.hasOwn(shell, 'summary')) {
    const summary = keys(shell.summary, ['line1', 'line2', 'attribution']);
    lines(summary);
    validateShellAttribution(summary.attribution);
  }
  requireValue(utf8.encode(canonicalShell(shell)).length <= SHELL_BYTES);
}
/** The write owner supplies attribution; action JSON cannot claim an author or a time. */
export function applyShellAction(
  base: unknown,
  action: string,
  input: unknown,
  attribution: ShellAttribution,
): Shell {
  validateShell(base);
  const next = structuredClone(base);
  const body = object(input);
  if (action === 'summary.set') {
    keys(body, ['line1', 'line2']);
    lines(body);
    validateShellAttribution(attribution);
    next.summary = { line1: body.line1 as string, line2: body.line2 as string, attribution };
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
