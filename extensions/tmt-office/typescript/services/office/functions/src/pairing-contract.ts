import { createHash } from 'node:crypto';

export type PairingCode =
  | 'INVALID_ARGUMENT'
  | 'UNAUTHENTICATED'
  | 'PERMISSION_DENIED'
  | 'PAIRING_UNAVAILABLE'
  | 'PAIRING_CONFLICT'
  | 'RETRY_LATER'
  | 'UNAVAILABLE';

export class PairingError extends Error {
  constructor(readonly code: PairingCode) {
    super(code);
  }
}

export interface Approval {
  version: 1;
  pairingId: string;
  worldId: string;
  installationId: string;
  identityId: string;
  installationLabel: string;
  identityLabel: string;
  capabilities: ['layout.read'] | ['layout.read', 'layout.write'];
}

export interface OwnerApproval {
  request: Approval;
  replacesPrincipalUid?: string;
}

export interface Renewal {
  version: 1;
  pairingId: string;
  grantExpiresAt: number;
}

export const APPROVAL_MS = 5 * 60_000;
export const GRANT_MS = 24 * 60 * 60_000;
export const RENEWAL_WINDOW_MS = 5 * 60_000;
export const CLAIM_INTERVAL_MS = 5000;
export const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
export const PAIRING_ID = /^[0-9a-f]{64}$/;
export const PRINCIPAL_UID =
  /^office-agent:[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
export const MAX_TIMESTAMP_MS = 8_640_000_000_000_000;

export function object(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value))
    throw new PairingError('INVALID_ARGUMENT');
  return value as Record<string, unknown>;
}

export function exact(value: Record<string, unknown>, keys: string[]): void {
  if (Object.keys(value).length !== keys.length || keys.some((key) => !Object.hasOwn(value, key)))
    throw new PairingError('INVALID_ARGUMENT');
}

function matches(value: unknown, pattern: RegExp): value is string {
  return typeof value === 'string' && pattern.test(value);
}

export function isTimestampMillis(value: unknown): value is number {
  return (
    typeof value === 'number' &&
    Number.isSafeInteger(value) &&
    value > 0 &&
    value <= MAX_TIMESTAMP_MS
  );
}

function label(value: unknown): value is string {
  return (
    typeof value === 'string' &&
    [...value].length <= 80 &&
    value.trim().length > 0 &&
    !/[\p{Cc}\p{Cs}]/u.test(value)
  );
}

export function parseApproval(input: unknown): Approval {
  const value = object(input);
  exact(value, [
    'version',
    'pairingId',
    'worldId',
    'installationId',
    'identityId',
    'installationLabel',
    'identityLabel',
    'capabilities',
  ]);
  const capabilities = value.capabilities;
  if (
    value.version !== 1 ||
    !matches(value.pairingId, PAIRING_ID) ||
    !matches(value.worldId, /^[a-zA-Z0-9]{20}$/) ||
    !matches(value.installationId, UUID) ||
    !matches(value.identityId, UUID) ||
    !label(value.installationLabel) ||
    !label(value.identityLabel) ||
    !Array.isArray(capabilities) ||
    capabilities[0] !== 'layout.read' ||
    !(
      capabilities.length === 1 ||
      (capabilities.length === 2 && capabilities[1] === 'layout.write')
    )
  )
    throw new PairingError('INVALID_ARGUMENT');
  return {
    version: 1,
    pairingId: value.pairingId,
    worldId: value.worldId,
    installationId: value.installationId,
    identityId: value.identityId,
    installationLabel: value.installationLabel,
    identityLabel: value.identityLabel,
    capabilities: capabilities.length === 1 ? ['layout.read'] : ['layout.read', 'layout.write'],
  };
}

/** Owner-only selection kept outside the strict native request contract. */
export function parseOwnerApproval(input: unknown): OwnerApproval {
  const value = object(input);
  const hasReplacement = Object.hasOwn(value, 'replacesPrincipalUid');
  const replacesPrincipalUid = hasReplacement ? value.replacesPrincipalUid : undefined;
  const requestValue = { ...value };
  delete requestValue.replacesPrincipalUid;
  const request = parseApproval(requestValue);
  if (
    hasReplacement &&
    (typeof replacesPrincipalUid !== 'string' || !PRINCIPAL_UID.test(replacesPrincipalUid))
  )
    throw new PairingError('INVALID_ARGUMENT');
  return replacesPrincipalUid === undefined
    ? { request }
    : { request, replacesPrincipalUid: replacesPrincipalUid as string };
}

export function parseClaim(input: unknown): string {
  const value = object(input);
  exact(value, ['version', 'secret']);
  if (value.version !== 1 || !matches(value.secret, /^[A-Za-z0-9_-]{43}$/))
    throw new PairingError('INVALID_ARGUMENT');
  const bytes = Buffer.from(value.secret, 'base64url');
  if (bytes.length !== 32 || bytes.toString('base64url') !== value.secret)
    throw new PairingError('INVALID_ARGUMENT');
  return createHash('sha256').update(bytes).digest('hex');
}

export function parseRenewal(input: unknown): Renewal {
  const value = object(input);
  exact(value, ['version', 'pairingId', 'grantExpiresAt']);
  if (
    value.version !== 1 ||
    !matches(value.pairingId, PAIRING_ID) ||
    !isTimestampMillis(value.grantExpiresAt)
  )
    throw new PairingError('INVALID_ARGUMENT');
  return {
    version: 1,
    pairingId: value.pairingId,
    grantExpiresAt: value.grantExpiresAt,
  };
}

export type Revocation =
  | { kind: 'pairing'; pairingId: string }
  | { kind: 'proof'; pairingId: string }
  | { kind: 'ownerApproval'; request: Approval };

export function parseRevocation(input: unknown): Revocation {
  const value = object(input);
  if (Object.hasOwn(value, 'secret')) return { kind: 'proof', pairingId: parseClaim(value) };
  if (Object.hasOwn(value, 'publicApproval')) {
    exact(value, ['version', 'publicApproval']);
    if (value.version !== 1) throw new PairingError('INVALID_ARGUMENT');
    return { kind: 'ownerApproval', request: parseApproval(value.publicApproval) };
  }
  exact(value, ['version', 'pairingId']);
  if (value.version !== 1 || !matches(value.pairingId, PAIRING_ID))
    throw new PairingError('INVALID_ARGUMENT');
  return { kind: 'pairing', pairingId: value.pairingId };
}
