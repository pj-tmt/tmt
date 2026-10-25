/** Hosted public configuration is a corruption-checked carrier, never an access grant. */
import { base64url } from './canonical-bytes.js';
import { ClientError, RefusalError } from './operations.js';

export interface PublicWebConfig {
  readonly apiKey: string;
  readonly appId: string;
  readonly authDomain: string;
  readonly projectId: string;
}
/** Syntax/checksum-validated public metadata, never Auth or page authority. */
export interface HostedReference {
  readonly origin: string;
  readonly publicWebConfig: PublicWebConfig;
  readonly hostingSite?: string;
}
const encoder = new TextEncoder();
const decoder = new TextDecoder('utf-8', { fatal: true });
const LINK_BYTES = 8192;
export const HOSTED_BOOTSTRAP_MS = 10_000;
/** The origin's own public descriptor grants no space or operations authority. */
export async function originLink(origin: string, fetcher: typeof fetch): Promise<string> {
  const controller = new AbortController();
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    valid(/^https:\/\/[a-z0-9](?:[a-z0-9-]{0,28}[a-z0-9])?\.web\.app$/.test(origin));
    const deadline = new Promise<never>((_, reject) => {
      timer = setTimeout(() => {
        controller.abort();
        reject(new RefusalError('REMOTE_INPUT_INVALID'));
      }, HOSTED_BOOTSTRAP_MS);
    });
    return await Promise.race([
      deadline,
      (async () => {
        const row = await readHostedConfig(origin, fetcher, controller.signal);
        const config = parsePublicWebConfig({
          apiKey: row.apiKey,
          appId: row.appId,
          authDomain: row.authDomain,
          projectId: row.projectId,
        });
        const site = new URL(origin).hostname.slice(0, -'.web.app'.length);
        const body = {
          kind: 'firestore',
          publicWebConfig: config,
          version: 1,
          ...(site === config.projectId ? {} : { hostingSite: site }),
        };
        const descriptor = canonical({ ...body, checksum: await checksum(body) });
        valid(!controller.signal.aborted);
        return `${origin}/pair#${base64url(encoder.encode(descriptor))}`;
      })(),
    ]);
  } catch {
    throw new RefusalError('REMOTE_INPUT_INVALID');
  } finally {
    clearTimeout(timer);
    controller.abort();
  }
}
function valid(condition: boolean): asserts condition {
  if (!condition) throw new RefusalError('REMOTE_INPUT_INVALID');
}
function object(value: unknown, keys: readonly string[]): Record<string, unknown> {
  valid(typeof value === 'object' && value !== null && !Array.isArray(value));
  const record = value as Record<string, unknown>;
  valid(Object.keys(record).sort().join(',') === [...keys].sort().join(','));
  return record;
}
export function parsePublicWebConfig(value: unknown): PublicWebConfig {
  const r = object(value, ['apiKey', 'appId', 'authDomain', 'projectId']);
  valid(typeof r.projectId === 'string' && /^[a-z][a-z0-9-]{4,28}[a-z0-9]$/.test(r.projectId));
  valid(r.authDomain === `${r.projectId}.firebaseapp.com`);
  for (const key of ['apiKey', 'appId'])
    valid(typeof r[key] === 'string' && /^[\x20-\x7e]{1,256}$/.test(r[key] as string));
  return Object.freeze(r) as unknown as PublicWebConfig;
}
function canonical(value: unknown): string {
  if (typeof value !== 'object' || value === null) return JSON.stringify(value);
  const r = value as Record<string, unknown>;
  return `{${Object.keys(r)
    .sort()
    .map((k) => `${JSON.stringify(k)}:${canonical(r[k])}`)
    .join(',')}}`;
}
async function checksum(value: unknown): Promise<string> {
  return Array.from(
    new Uint8Array(await crypto.subtle.digest('SHA-256', encoder.encode(canonical(value)))),
    (b) => b.toString(16).padStart(2, '0'),
  ).join('');
}

export async function verifyHostedLink(link: string, origin: string): Promise<HostedReference> {
  try {
    valid(typeof link === 'string' && encoder.encode(link).length <= LINK_BYTES);
    const url = new URL(link);
    valid(
      url.origin === origin &&
        url.protocol === 'https:' &&
        url.pathname === '/pair' &&
        url.search === '' &&
        url.username === '' &&
        url.password === '',
    );
    const fragment = url.hash.slice(1);
    valid(/^[A-Za-z0-9_-]+$/.test(fragment));
    const bytes = Uint8Array.from(atob(fragment.replace(/-/g, '+').replace(/_/g, '/')), (c) =>
      c.charCodeAt(0),
    );
    const text = decoder.decode(bytes);
    const raw: unknown = JSON.parse(text);
    const hasSite = typeof raw === 'object' && raw !== null && Object.hasOwn(raw, 'hostingSite');
    const r = object(raw, [
      'version',
      'kind',
      'publicWebConfig',
      'checksum',
      ...(hasSite ? ['hostingSite'] : []),
    ]);
    valid(
      r.version === 1 &&
        r.kind === 'firestore' &&
        typeof r.checksum === 'string' &&
        /^[a-f0-9]{64}$/.test(r.checksum),
    );
    const publicConfig = parsePublicWebConfig(r.publicWebConfig);
    let site = publicConfig.projectId;
    if (hasSite) {
      const selected = r.hostingSite;
      valid(
        typeof selected === 'string' &&
          /^[a-z0-9](?:[a-z0-9-]{0,28}[a-z0-9])?$/.test(selected) &&
          selected !== publicConfig.projectId,
      );
      site = selected;
    }
    valid(origin === `https://${site}.web.app`);
    const body = {
      kind: r.kind,
      publicWebConfig: publicConfig,
      version: r.version,
      ...(hasSite ? { hostingSite: site } : {}),
    };
    valid((await checksum(body)) === r.checksum && text === canonical(r));
    valid(link === `${origin}/pair#${base64url(bytes)}`);
    return Object.freeze({
      origin,
      publicWebConfig: publicConfig,
      ...(hasSite ? { hostingSite: site } : {}),
    });
  } catch {
    throw new RefusalError('REMOTE_INPUT_INVALID');
  }
}

/** A bounded same-origin read cannot follow a redirect into another configuration source. */
export async function confirmHostedConfig(
  expected: PublicWebConfig,
  origin: string,
  fetcher: typeof fetch,
  signal: AbortSignal,
): Promise<void> {
  const r = await readHostedConfig(origin, fetcher, signal);
  // Hosting may expose unrelated public configuration; never pass it into Firebase.
  for (const key of ['projectId', 'appId', 'authDomain'] as const) {
    if (r[key] !== expected[key])
      throw new ClientError(
        'unverifiable_response',
        'Hosted configuration does not match this remote link.',
      );
  }
}
/** One bounded response codec for origin-link construction and the unchanged connect check. */
async function readHostedConfig(
  origin: string,
  fetcher: typeof fetch,
  signal: AbortSignal,
): Promise<Record<string, unknown>> {
  const response = await fetcher(`${origin}/__/firebase/init.json`, {
    signal,
    redirect: 'error',
    credentials: 'omit',
    cache: 'no-store',
  });
  if (!response.ok || (response.url !== '' && response.url !== `${origin}/__/firebase/init.json`))
    throw new ClientError('unverifiable_response', 'Hosted configuration could not be confirmed.');
  const reader = response.body?.getReader();
  if (!reader)
    throw new ClientError('unverifiable_response', 'Hosted configuration could not be confirmed.');
  let text = '';
  let size = 0;
  const streamDecoder = new TextDecoder('utf-8', { fatal: true });
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (signal.aborted) throw new ClientError('timeout', 'Hosted configuration check timed out.');
      if (done) break;
      size += value.byteLength;
      if (size > LINK_BYTES)
        throw new ClientError('unverifiable_response', 'Hosted configuration exceeds its bound.');
      text += streamDecoder.decode(value, { stream: true });
    }
    text += streamDecoder.decode();
    const actual: unknown = JSON.parse(text);
    if (typeof actual !== 'object' || actual === null || Array.isArray(actual))
      throw new Error('invalid');
    return actual as Record<string, unknown>;
  } finally {
    await reader.cancel();
    reader.releaseLock();
  }
}
