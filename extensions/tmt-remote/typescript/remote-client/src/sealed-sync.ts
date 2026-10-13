/** Proposed extension-scoped sealed-document ports. No provider, credentials or factory. */
import { parsePublicWebConfig, type PublicWebConfig } from './firebase-bootstrap.js';
import { verifyHostedDeclaration, type HostedDeclaration } from './firebase-declaration.js';
import { parseSyncMapping, type SyncMapping } from './sealed-sync-mapping.js';
export type { SyncMapping, SyncQueryMapping, SyncDiscoveryMapping } from './sealed-sync-mapping.js';
import { RefusalError } from './operations.js';

export interface SyncIdentity {
  readonly projectId: string;
  readonly uid: string;
}
/** Secret token is transient; never serialize it, log it or put it in an operation identity. */
export interface SyncToken extends SyncIdentity {
  readonly token: string;
  readonly expiresAtMs: number;
}
export type SyncAuthState = 'expired' | 'sign-in-needed' | 'unavailable' | 'cancelled';
export interface SyncAuthFailure {
  readonly code: 'REMOTE_STATE_UNAVAILABLE';
  readonly state: SyncAuthState;
}
/** Identity is already authenticated by the injected owner. Token acquisition never changes uid. */
export interface SyncAuthPort {
  readonly identity: SyncIdentity;
  token(options: SyncCall): Promise<SyncToken | SyncAuthFailure>;
}
export interface SyncCall {
  readonly signal: AbortSignal;
  readonly deadlineAtMs: number;
}
export interface SyncResource {
  readonly sync?: SyncMapping;
  readonly name: string;
  readonly path: string;
  readonly maxObjectBytes: number;
  readonly chunkBytes?: number;
}
export interface SyncContext {
  readonly config: PublicWebConfig;
  readonly extension: string;
  readonly declarationDigest: string;
  readonly space: string;
  readonly uid: string;
  readonly resources: readonly SyncResource[];
  readonly auth: SyncAuthPort;
}
export interface SyncTarget {
  readonly resource: string;
  readonly id: string;
}
/** Canonical restricted REST Document.fields bytes; sealed content stays encrypted. */
export interface SealedDocument {
  readonly target: SyncTarget;
  readonly bytes: Uint8Array;
  readonly digest: string;
}
export interface SyncCursor {
  readonly stream: string;
  readonly seq: number;
}
export type SyncValue =
  | { readonly stringValue: string }
  | { readonly integerValue: string }
  | { readonly booleanValue: boolean }
  | { readonly nullValue: null }
  | { readonly timestampValue: string }
  | { readonly bytesValue: string };
export interface SyncQuery {
  readonly resource: string;
  readonly scope: Readonly<Record<string, SyncValue>>;
  readonly stream: string;
  readonly cursor?: SyncCursor;
}
/** Page-level interest with no seed stream or delivery cursor. */
export interface SyncInterest {
  readonly resource: string;
  readonly scope: Readonly<Record<string, SyncValue>>;
}
export interface SyncDiscovery {
  readonly resource: string;
  readonly shape: string;
  readonly continuation?: string;
  readonly scope: Readonly<Record<string, SyncValue>>;
}
export type SyncQueryRequest =
  | { readonly shape: 'stream'; readonly query: SyncQuery }
  | { readonly shape: 'discovery'; readonly query: SyncDiscovery };
export type SyncMutation =
  | { readonly mode: 'create' | 'put'; readonly document: SealedDocument }
  | { readonly mode: 'delete'; readonly target: SyncTarget };
export interface SyncPage {
  readonly documents: readonly SealedDocument[];
  readonly cursor?: SyncCursor;
}
export interface SyncDiscoveryPage {
  readonly documents: readonly SealedDocument[];
  /** Absent means a complete run; otherwise an opaque stateless token, never a stream cursor. */
  readonly continuation?: string;
}
export type SyncQueryPage =
  | { readonly shape: 'stream'; readonly page: SyncPage }
  | { readonly shape: 'discovery'; readonly page: SyncDiscoveryPage };
export type SyncChange = { readonly kind: 'changed' } | { readonly kind: 'catch-up-required' };
export interface SyncSubscription {
  next(call: SyncCall): Promise<SyncChange>;
  close(): void;
}
export type SyncFailure =
  | {
      readonly code: 'REMOTE_STATE_UNAVAILABLE';
      readonly state: 'catch-up-required' | 'conflict' | 'unsupported';
    }
  | {
      readonly code:
        | 'REMOTE_INPUT_INVALID'
        | 'REMOTE_SCOPE_DENIED'
        | 'REMOTE_STATE_UNAVAILABLE'
        | 'REMOTE_RATE_LIMITED';
    }
  | {
      readonly code: 'REMOTE_BUDGET_EXHAUSTED';
      readonly resetAtMs: number;
      readonly retryAfterMs: number;
    };
/** Rejections use SyncFailure codes; they are not machine-signed device proofs. */
export type SyncPublish =
  | { readonly kind: 'confirmed' }
  | { readonly kind: 'already-present' }
  | { readonly kind: 'unknown' };
/** Later browser/native transports implement this bound port; no operations authority. */
export interface SealedSyncPort {
  readonly context: SyncContext;
  read(target: SyncTarget, call: SyncCall): Promise<SealedDocument | undefined>;
  query(query: SyncQueryRequest, call: SyncCall): Promise<SyncQueryPage>;
  subscribe(interest: SyncInterest, call: SyncCall): Promise<SyncSubscription>;
  /** Only pre-effect refusal rejects; a possibly applied write is unknown, including cancellation. */
  publish(mutation: SyncMutation, call: SyncCall): Promise<SyncPublish>;
  close(): void;
}
const identity = /^[A-Za-z0-9_+-]{1,128}$/;
const documentId = /^[A-Za-z0-9_+-]{1,256}$/;
function valid(condition: boolean): asserts condition {
  if (!condition) throw new RefusalError('REMOTE_INPUT_INVALID');
}
/** Caller supplies bootstrap-verified public config. This pure binding does not attest an origin. */
export async function bindSyncContext(
  config: PublicWebConfig,
  declaration: HostedDeclaration,
  space: string,
  auth: SyncAuthPort,
): Promise<SyncContext> {
  config = parsePublicWebConfig(config);
  await verifyHostedDeclaration(declaration);
  valid(identity.test(space) && identity.test(auth.identity.uid));
  if (config.projectId !== auth.identity.projectId) throw new RefusalError('REMOTE_SCOPE_DENIED');
  const row = JSON.parse(declaration.bytes) as {
    extension: string;
    resources: {
      name: string;
      path: string;
      limits: { maxObjectBytes: number; chunkBytes?: number };
      sync?: unknown;
    }[];
  };
  for (const [i, resource] of row.resources.entries()) {
    const segments = resource.path.split('/').length;
    valid(segments <= 7 && segments % 2 === 1);
    valid(
      !row.resources
        .slice(0, i)
        .some(
          (other) =>
            resource.path === other.path ||
            resource.path.startsWith(`${other.path}/`) ||
            other.path.startsWith(`${resource.path}/`),
        ),
    );
  }
  const resources = row.resources.map((r) =>
    Object.freeze({
      name: r.name,
      path: r.path,
      maxObjectBytes: r.limits.maxObjectBytes,
      ...(r.sync === undefined ? {} : { sync: parseSyncMapping(r.sync) }),
      ...(r.limits.chunkBytes === undefined ? {} : { chunkBytes: r.limits.chunkBytes }),
    }),
  );
  return Object.freeze({
    config: Object.freeze({ ...config }),
    extension: row.extension,
    declarationDigest: declaration.digest,
    space,
    uid: auth.identity.uid,
    resources: Object.freeze(resources),
    auth,
  });
}
/** A document target cannot escape its declared collection or select an operations route. */
export function syncDocumentPath(context: SyncContext, target: SyncTarget): string {
  const resource = context.resources.find((r) => r.name === target.resource);
  valid(resource !== undefined && documentId.test(target.id));
  return `x/${context.extension}/${resource.path}/${target.id}`;
}

export interface SealedObjectManifest {
  readonly original: string;
  readonly count: number;
  readonly bytes: number;
  readonly digest: string;
  readonly chunkBytes: number;
}
/** Pure complete-or-absent validation; never returns a truncated object. */
export async function verifySealedObject(
  original: string,
  manifest: SealedObjectManifest,
  chunks: readonly Uint8Array[],
  maxObjectBytes: number,
): Promise<void> {
  valid(documentId.test(original) && manifest.original === original);
  valid(Number.isSafeInteger(maxObjectBytes) && maxObjectBytes > 0 && maxObjectBytes <= 16_779_264);
  valid(
    Number.isSafeInteger(manifest.chunkBytes) &&
      manifest.chunkBytes > 0 &&
      manifest.chunkBytes <= maxObjectBytes,
  );
  valid(
    Number.isSafeInteger(manifest.bytes) && manifest.bytes >= 0 && manifest.bytes <= maxObjectBytes,
  );
  valid(
    Number.isSafeInteger(manifest.count) &&
      manifest.count >= 0 &&
      manifest.count <= 513 &&
      manifest.count === Math.ceil(manifest.bytes / manifest.chunkBytes),
  );
  valid(chunks.length === manifest.count && /^[a-f0-9]{64}$/.test(manifest.digest));
  const bytes = new Uint8Array(manifest.bytes);
  let offset = 0;
  for (let i = 0; i < chunks.length; i++) {
    const chunk = chunks[i]!;
    valid(chunk.byteLength === Math.min(manifest.chunkBytes, manifest.bytes - offset));
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  const digest = Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', bytes)), (b) =>
    b.toString(16).padStart(2, '0'),
  ).join('');
  valid(digest === manifest.digest);
}

export {
  validateSyncDocument,
  validateSyncFields,
  validateSyncQuery,
  validateSyncInterest,
  validateSyncDiscovery,
  validateSyncMutation,
} from './sealed-sync-codec.js';
