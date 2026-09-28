import { boundedText, exactRecord } from '../contracts/record.js';
import { validImmutableArtDigest } from '../rendering/immutable-art-reference.js';
import { decodePropPack } from './prop-contract.js';
import type { CatalogPack } from './prop-contract.js';

export const PROP_FILE_BYTES = 512 * 1024;
export const PROP_CATALOG_PAGE_BYTES = 32 * 1024;
export interface PropCatalogPage {
  revision: number;
  entries: { digest: string; label: string }[];
  excluded: { digest: string; reason: string }[];
  nextCursor: string | null;
}
export interface PropInstallInput {
  expectedRevision: number;
  document: string;
}
export interface PropInstallReceipt {
  revision: number;
  digest: string;
  changed: boolean;
}
export interface PropCatalogPort {
  list(cursor?: string, signal?: AbortSignal): Promise<PropCatalogPage>;
  load(digest: string, signal?: AbortSignal): Promise<CatalogPack>;
  install(input: PropInstallInput, signal?: AbortSignal): Promise<PropInstallReceipt>;
}

function revision(value: unknown): number {
  if (!Number.isSafeInteger(value) || Number(value) < 0)
    throw new Error('Invalid catalog revision.');
  return value as number;
}

export function propCatalogCursor(value: unknown): string {
  if (typeof value !== 'string' || !/^[A-Za-z0-9_-]{1,256}$/.test(value))
    throw new Error('Invalid catalog cursor.');
  return value;
}

export function decodePropCatalogPage(value: unknown): PropCatalogPage {
  const page = exactRecord(
    value,
    ['revision', 'entries', 'excluded', 'nextCursor'],
    'prop catalog page'
  );
  if (
    !Array.isArray(page.entries) ||
    page.entries.length > 20 ||
    !Array.isArray(page.excluded) ||
    page.excluded.length > 64
  )
    throw new Error('Invalid catalog page size.');
  const seen = new Set<string>();
  const digest = (value: unknown) => {
    if (!validImmutableArtDigest(value) || seen.has(value))
      throw new Error('Invalid or duplicate catalog digest.');
    seen.add(value);
    return value;
  };
  const entries = page.entries.map((value) => {
    const entry = exactRecord(value, ['digest', 'label'], 'catalog entry');
    if (!boundedText(entry.label, 80)) throw new Error('Invalid catalog label.');
    return { digest: digest(entry.digest), label: entry.label };
  });
  const excluded = page.excluded.map((value) => {
    const entry = exactRecord(value, ['digest', 'reason'], 'excluded catalog entry');
    if (!['oversized', 'digestMismatch', 'invalidDocument'].includes(String(entry.reason)))
      throw new Error('Invalid catalog exclusion.');
    return { digest: digest(entry.digest), reason: entry.reason as string };
  });
  return {
    revision: revision(page.revision),
    entries,
    excluded,
    nextCursor: page.nextCursor === null ? null : propCatalogCursor(page.nextCursor),
  };
}

export function decodePropInstallReceipt(value: unknown): PropInstallReceipt {
  const receipt = exactRecord(value, ['revision', 'digest', 'changed'], 'prop install receipt');
  if (!validImmutableArtDigest(receipt.digest) || typeof receipt.changed !== 'boolean')
    throw new Error('Invalid prop install receipt.');
  return { revision: revision(receipt.revision), digest: receipt.digest, changed: receipt.changed };
}

/** Hash exact submitted bytes under the native versioned, length-framed identity. */
export async function propDocumentDigest(input: PropInstallInput): Promise<string> {
  revision(input.expectedRevision);
  const bytes = new TextEncoder().encode(input.document);
  if (
    bytes.length > PROP_FILE_BYTES ||
    input.document.startsWith('\ufeff') ||
    /[\uD800-\uDFFF]/u.test(input.document)
  )
    throw new Error('Invalid prop file encoding or size.');
  const pack = decodePropPack(JSON.parse(input.document));
  if (pack.formatVersion === 1 && bytes.length > 128 * 1024)
    throw new Error('V1 prop file exceeds its byte limit.');
  const domain = new TextEncoder().encode(`TMT-OFFICE-PROP-PACK-V${pack.formatVersion}\0`);
  const framed = new Uint8Array(domain.length + 8 + bytes.length);
  framed.set(domain);
  new DataView(framed.buffer).setBigUint64(domain.length, BigInt(bytes.length));
  framed.set(bytes, domain.length + 8);
  const hash = new Uint8Array(await crypto.subtle.digest('SHA-256', framed));
  return `sha256:${Array.from(hash, (byte) => byte.toString(16).padStart(2, '0')).join('')}`;
}
