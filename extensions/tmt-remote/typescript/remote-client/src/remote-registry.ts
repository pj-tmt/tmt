/** Public references only; keys, pairings, credentials, drafts and intents never enter a catalog. */
import { base64url } from './canonical-bytes.js';
import { verifyHostedLink } from './firebase-bootstrap.js';
import { readRegistry, writeRegistry } from './browser-store.js';

export interface RemoteReference {
  readonly name: string;
  readonly kind: 'local' | 'firestore';
  readonly url: string;
  readonly project: string | null;
}
export interface RemoteCatalog {
  readonly version: 1;
  readonly entries: readonly RemoteReference[];
}
export interface NavigationHint {
  readonly spaceId: string;
  readonly pageId: string;
}
export interface RegistryHandoff {
  readonly catalog: RemoteCatalog;
  readonly hint?: NavigationHint;
}
export class RemoteRegistryError extends Error {
  constructor(
    readonly code:
      | 'REMOTE_SELECTOR_INVALID'
      | 'REMOTE_SELECTOR_CONFLICT'
      | 'REMOTE_SELECTOR_UNAVAILABLE',
    message: string,
  ) {
    super(message);
  }
}
const LIMIT = 8192;
const encoder = new TextEncoder();
const decoder = new TextDecoder('utf-8', { fatal: true });
const empty: RemoteCatalog = { version: 1, entries: [] };
function valid(
  condition: boolean,
  message = 'The remote name or link is invalid',
): asserts condition {
  if (!condition) throw new RemoteRegistryError('REMOTE_SELECTOR_INVALID', message);
}
function fields(value: unknown, keys: readonly string[]): Record<string, unknown> {
  valid(typeof value === 'object' && value !== null && !Array.isArray(value));
  const object = value as Record<string, unknown>;
  valid(Object.keys(object).sort().join(',') === [...keys].sort().join(','));
  return object;
}
function name(value: unknown): asserts value is string {
  valid(typeof value === 'string' && /^[a-z0-9-]{1,63}$/.test(value) && value !== 'local');
}
function target(entry: RemoteReference): string {
  return new URL(entry.url).href;
}
export async function reference(alias: string, address: string): Promise<RemoteReference> {
  name(alias);
  valid(typeof address === 'string' && address.length <= LIMIT);
  let url: URL;
  try {
    url = new URL(address);
  } catch {
    valid(false);
  }
  if (url.protocol === 'https:' && url.hostname.endsWith('.web.app') && url.pathname === '/pair') {
    const config = await verifyHostedLink(address, url.origin).catch(() => {
      valid(false);
    });
    return { name: alias, kind: 'firestore', url: address, project: config.projectId };
  }
  valid(
    address.length <= 2048 &&
      /^https?:\/\/[A-Za-z0-9.:-]+\/[A-Za-z0-9/_-]*$/.test(address) &&
      address.endsWith('/') &&
      !url.search &&
      !url.hash &&
      !url.username &&
      !url.password,
  );
  return { name: alias, kind: 'local', url: address, project: null };
}
export async function parseCatalog(value: unknown): Promise<RemoteCatalog> {
  const row = fields(value, ['version', 'entries']);
  valid(row.version === 1 && Array.isArray(row.entries));
  valid(
    row.entries.length <= 16,
    'This browser can store at most 16 remotes. Remove one before adding another.',
  );
  const entries: RemoteReference[] = [];
  for (const raw of row.entries as unknown[]) {
    const r = fields(raw, ['name', 'kind', 'url', 'project']);
    const entry = await reference(r.name as string, r.url as string);
    valid(entry.kind === r.kind && entry.project === r.project);
    valid(!entries.some((old) => old.name === entry.name || target(old) === target(entry)));
    entries.push(Object.freeze(entry));
  }
  const catalog = { version: 1 as const, entries: Object.freeze(entries) };
  valid(
    encoder.encode(json(catalog)).length <= LIMIT,
    'The remotes exceed this browser’s 8 KiB storage limit. Remove one before adding another.',
  );
  return Object.freeze(catalog);
}
function json(catalog: RemoteCatalog): string {
  return JSON.stringify(catalog);
}
function decode(value: string): unknown {
  valid(/^[A-Za-z0-9_-]+$/.test(value));
  try {
    const bytes = Uint8Array.from(atob(value.replace(/-/g, '+').replace(/_/g, '/')), (c) =>
      c.charCodeAt(0),
    );
    valid(base64url(bytes) === value);
    return JSON.parse(decoder.decode(bytes));
  } catch {
    valid(false);
  }
}
function hint(value: unknown): NavigationHint {
  const r = fields(value, ['spaceId', 'pageId']);
  valid(typeof r.spaceId === 'string' && /^[a-z2-7]{32}$/.test(r.spaceId));
  valid(
    typeof r.pageId === 'string' &&
      /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(r.pageId),
  );
  return { spaceId: r.spaceId, pageId: r.pageId };
}
export async function parseHandoff(fragment: string): Promise<RegistryHandoff> {
  valid(fragment.startsWith('#') && encoder.encode(fragment).length <= LIMIT);
  const params = new URLSearchParams(fragment.slice(1));
  const keys = [...params.keys()];
  valid(
    keys.length === new Set(keys).size &&
      keys.every((key) => ['tmt-remotes', 'tmt-page'].includes(key)),
  );
  const raw = params.get('tmt-remotes');
  valid(raw !== null);
  const value = decode(raw);
  const catalog = await parseCatalog(value);
  valid(base64url(encoder.encode(json(catalog))) === raw);
  const page = params.get('tmt-page');
  const checkedHint = page === null ? undefined : hint(decode(page));
  if (checkedHint) valid(base64url(encoder.encode(JSON.stringify(checkedHint))) === page);
  return { catalog, ...(checkedHint ? { hint: checkedHint } : {}) };
}
export async function buildHandoff(
  destination: RemoteReference,
  value: unknown,
  page?: unknown,
): Promise<string> {
  const checked = await reference(destination.name, destination.url);
  valid(checked.kind === destination.kind && checked.project === destination.project);
  if (checked.kind === 'firestore') return checked.url; // B1 has no hosted catalog receiver.
  const catalog = await parseCatalog(value);
  const params = new URLSearchParams({
    'tmt-remotes': base64url(encoder.encode(json(catalog))),
  });
  if (page !== undefined)
    params.set('tmt-page', base64url(encoder.encode(JSON.stringify(hint(page)))));
  valid(
    encoder.encode(`#${params}`).length <= LIMIT,
    'The remote list is too large to open together. Remove an entry before opening this remote.',
  );
  return `${checked.url}#${params}`;
}
export interface ImportRow {
  readonly entry: RemoteReference;
  readonly status: 'add' | 'collision' | 'duplicate';
  readonly suggestedName: string;
  readonly existingName?: string;
}
export interface ImportPlan {
  readonly incoming: RemoteCatalog;
  readonly current: RemoteCatalog;
  readonly rows: readonly ImportRow[];
}
export async function planImport(incoming: unknown, current: unknown): Promise<ImportPlan> {
  const next = await parseCatalog(incoming),
    before = await parseCatalog(current);
  const used = new Set([...before.entries, ...next.entries].map((entry) => entry.name));
  const rows = next.entries.map((entry): ImportRow => {
    const same = before.entries.find((old) => target(old) === target(entry));
    if (same)
      return { entry, status: 'duplicate', suggestedName: entry.name, existingName: same.name };
    if (!before.entries.some((old) => old.name === entry.name))
      return { entry, status: 'add', suggestedName: entry.name };
    let n = 2;
    let proposed: string;
    do {
      const suffix = `-${n++}`;
      proposed = entry.name.slice(0, 63 - suffix.length) + suffix;
    } while (used.has(proposed));
    used.add(proposed);
    return { entry, status: 'collision', suggestedName: proposed };
  });
  return { incoming: next, current: before, rows };
}
export async function mergeImport(
  plan: ImportPlan,
  choices: readonly (string | null)[],
): Promise<RemoteCatalog> {
  const checked = await planImport(plan.incoming, plan.current);
  valid(choices.length === checked.rows.length);
  const entries = [...checked.current.entries];
  for (const [index, row] of checked.rows.entries()) {
    const alias = choices[index];
    if (alias === null || row.status === 'duplicate') continue;
    name(alias);
    if (entries.some((old) => old.name === alias))
      throw new RemoteRegistryError(
        'REMOTE_SELECTOR_CONFLICT',
        `You already have a remote named ${alias}`,
      );
    entries.push({ ...row.entry, name: alias });
  }
  return parseCatalog({ version: 1, entries });
}
async function snapshot(): Promise<{ raw: unknown; catalog: RemoteCatalog }> {
  try {
    const raw = await readRegistry();
    valid(
      raw === undefined ||
        (typeof raw === 'string' && raw.length <= LIMIT && encoder.encode(raw).length <= LIMIT),
    );
    const catalog = raw === undefined ? empty : await parseCatalog(JSON.parse(raw));
    valid(raw === undefined || raw === json(catalog));
    return { raw, catalog };
  } catch {
    throw new RemoteRegistryError('REMOTE_SELECTOR_UNAVAILABLE', 'The remotes could not be read');
  }
}
async function commit(raw: unknown, catalog: RemoteCatalog): Promise<void> {
  try {
    await writeRegistry(raw, json(catalog));
  } catch (error) {
    throw new RemoteRegistryError(
      error instanceof Error && error.message === 'Registry changed'
        ? 'REMOTE_SELECTOR_CONFLICT'
        : 'REMOTE_SELECTOR_UNAVAILABLE',
      'The remotes could not be saved. Check the list again.',
    );
  }
}
export const remoteRegistry = {
  parseHandoff,
  async list(): Promise<RemoteCatalog> {
    return (await snapshot()).catalog;
  },
  async planImport(incoming: unknown): Promise<ImportPlan> {
    return planImport(incoming, (await snapshot()).catalog);
  },
  async confirmImport(plan: ImportPlan, choices: readonly (string | null)[]): Promise<void> {
    const next = await mergeImport(plan, choices),
      latest = await snapshot();
    if (json(latest.catalog) !== json(plan.current))
      throw new RemoteRegistryError(
        'REMOTE_SELECTOR_CONFLICT',
        'The remotes changed. Check the list again.',
      );
    await commit(latest.raw, next);
  },
  async add(alias: string, url: string): Promise<void> {
    const entry = await reference(alias, url),
      latest = await snapshot();
    const plan = await planImport({ version: 1, entries: [entry] }, latest.catalog);
    valid(plan.rows[0]!.status !== 'duplicate', `Already added as ${plan.rows[0]!.existingName}`);
    await remoteRegistry.confirmImport(plan, [alias]);
  },
  async rename(previous: string, alias: string): Promise<void> {
    name(alias);
    const latest = await snapshot();
    valid(latest.catalog.entries.some((entry) => entry.name === previous));
    valid(
      !latest.catalog.entries.some((entry) => entry.name === alias && entry.name !== previous),
      `You already have a remote named ${alias}`,
    );
    const next = await parseCatalog({
      version: 1,
      entries: latest.catalog.entries.map((entry) =>
        entry.name === previous ? { ...entry, name: alias } : entry,
      ),
    });
    await commit(latest.raw, next);
  },
  async remove(alias: string): Promise<void> {
    const latest = await snapshot();
    valid(latest.catalog.entries.some((entry) => entry.name === alias));
    await commit(latest.raw, {
      version: 1,
      entries: latest.catalog.entries.filter((entry) => entry.name !== alias),
    });
  },
  async navigation(entry: RemoteReference, page?: NavigationHint): Promise<string> {
    return buildHandoff(entry, await remoteRegistry.list(), page);
  },
};
