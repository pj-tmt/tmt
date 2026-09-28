import { WORLD_ID_PATTERN } from '../worlds/world-contract.js';

export type PairingCapabilities = ['layout.read'] | ['layout.read', 'layout.write'];

export interface PairingRequest {
  version: 1;
  pairingId: string;
  worldId: string;
  installationId: string;
  identityId: string;
  installationLabel: string;
  identityLabel: string;
  capabilities: PairingCapabilities;
}

export interface ApprovedPairing extends PairingRequest {
  principalUid: string;
  blockId: string;
  expiresAt: number;
}

export interface PairingPort {
  approve(
    request: PairingRequest,
    ownerUid: string,
    replacement?: PairingReplacement
  ): Promise<ApprovedPairing>;
  revoke(request: PairingRequest, ownerUid: string): Promise<void>;
}

/** Owner-selected intent, never part of the native request fragment. */
export interface PairingReplacement {
  principalUid: string;
  blockId: string;
}

export function validatePairingReplacement(value: PairingReplacement): void {
  if (
    !value.principalUid.startsWith('office-agent:') ||
    !UUID.test(value.principalUid.slice('office-agent:'.length)) ||
    !UUID.test(value.blockId)
  )
    invalid();
}

export class PairingActionError extends Error {
  constructor(readonly kind: 'denied' | 'unavailable' | 'conflict' | 'uncertain') {
    super(
      {
        denied: 'Pairing was denied. Check your account and current access.',
        unavailable: 'This request is unavailable or expired. Request a new pairing link.',
        conflict:
          'The assignment conflicts with an existing approval or transfer. Keep the same choice when retrying; otherwise request a new link.',
        uncertain:
          'The service could not confirm the result. Keep this link and retry the same action; a submitted change may already have completed.',
      }[kind]
    );
  }
}

const REQUEST_KEYS = [
  'version',
  'pairingId',
  'worldId',
  'installationId',
  'identityId',
  'installationLabel',
  'identityLabel',
  'capabilities',
] as const;
const APPROVED_KEYS = [...REQUEST_KEYS, 'principalUid', 'blockId', 'expiresAt'];
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const PAIRING_ID = /^[0-9a-f]{64}$/;
const WORLD_ID = new RegExp(`^${WORLD_ID_PATTERN}$`);
const FRAGMENT_PREFIX = 'tmt-pair=';
const MAX_DECODED_FRAGMENT_BYTES = 2048;
const MAX_FRAGMENT_PAYLOAD = Math.ceil((MAX_DECODED_FRAGMENT_BYTES * 4) / 3);
const MAX_DATE_MS = 8_640_000_000_000_000;

function invalid(): never {
  throw new Error('Invalid pairing input.');
}

function exact(value: Record<string, unknown>, keys: readonly string[]): void {
  if (Object.keys(value).length !== keys.length || keys.some((key) => !Object.hasOwn(value, key)))
    invalid();
}

function record(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) invalid();
  return value as Record<string, unknown>;
}

function label(value: unknown): value is string {
  return (
    typeof value === 'string' &&
    [...value].length <= 80 &&
    value.trim().length > 0 &&
    !/[\p{Cc}\p{Cs}]/u.test(value)
  );
}

function matches(value: unknown, pattern: RegExp): value is string {
  return typeof value === 'string' && pattern.test(value);
}

function capabilities(value: unknown): PairingCapabilities | undefined {
  if (!Array.isArray(value) || value[0] !== 'layout.read') return undefined;
  if (value.length === 1) return ['layout.read'];
  if (value.length === 2 && value[1] === 'layout.write') return ['layout.read', 'layout.write'];
  return undefined;
}

function parseRequest(value: unknown, worldId: string): PairingRequest {
  const input = record(value);
  exact(input, REQUEST_KEYS);
  const parsedCapabilities = capabilities(input.capabilities);
  if (
    input.version !== 1 ||
    !matches(input.pairingId, PAIRING_ID) ||
    !matches(input.worldId, WORLD_ID) ||
    input.worldId !== worldId ||
    !matches(input.installationId, UUID) ||
    !matches(input.identityId, UUID) ||
    !label(input.installationLabel) ||
    !label(input.identityLabel) ||
    !parsedCapabilities
  )
    invalid();
  return {
    version: 1,
    pairingId: input.pairingId,
    worldId: input.worldId,
    installationId: input.installationId,
    identityId: input.identityId,
    installationLabel: input.installationLabel,
    identityLabel: input.identityLabel,
    capabilities: parsedCapabilities,
  };
}

/** Canonical immutable request snapshot for one mounted action or transport call. */
export function snapshotPairingRequest(request: PairingRequest): PairingRequest {
  const parsed = parseRequest(request, request.worldId);
  return Object.freeze({
    ...parsed,
    capabilities: Object.freeze([...parsed.capabilities]),
  }) as PairingRequest;
}

function requestFields(value: Record<string, unknown>): Record<string, unknown> {
  const request: Record<string, unknown> = {};
  for (const key of REQUEST_KEYS) request[key] = value[key];
  return request;
}

function equalRequest(left: PairingRequest, right: PairingRequest): boolean {
  return (
    left.version === right.version &&
    left.pairingId === right.pairingId &&
    left.worldId === right.worldId &&
    left.installationId === right.installationId &&
    left.identityId === right.identityId &&
    left.installationLabel === right.installationLabel &&
    left.identityLabel === right.identityLabel &&
    left.capabilities.length === right.capabilities.length &&
    left.capabilities.every((capability, index) => capability === right.capabilities[index])
  );
}

function decodeBase64Url(value: string): Uint8Array {
  return Uint8Array.from(atob(value.replace(/-/g, '+').replace(/_/g, '/')), (character) =>
    character.charCodeAt(0)
  );
}

function encodeBase64Url(bytes: Uint8Array): string {
  return btoa(String.fromCharCode(...bytes))
    .replace(/\+/g, '-')
    .replace(/\//g, '_')
    .replace(/=+$/, '');
}

export function parsePairingFragment(fragment: string, worldId: string): PairingRequest {
  try {
    if (
      typeof fragment !== 'string' ||
      fragment.length > 1 + FRAGMENT_PREFIX.length + MAX_FRAGMENT_PAYLOAD
    )
      invalid();
    if (!matches(worldId, WORLD_ID)) invalid();
    const value = fragment.startsWith('#') ? fragment.slice(1) : fragment;
    if (!value.startsWith(FRAGMENT_PREFIX)) invalid();
    const encoded = value.slice(FRAGMENT_PREFIX.length);
    if (
      encoded.length === 0 ||
      encoded.length > MAX_FRAGMENT_PAYLOAD ||
      !/^[A-Za-z0-9_-]+$/.test(encoded)
    )
      invalid();
    const bytes = decodeBase64Url(encoded);
    if (bytes.length > MAX_DECODED_FRAGMENT_BYTES) invalid();
    const json = new TextDecoder('utf-8', { fatal: true }).decode(bytes);
    const canonical = new TextEncoder().encode(json);
    if (encodeBase64Url(canonical) !== encoded) invalid();
    return parseRequest(JSON.parse(json), worldId);
  } catch {
    invalid();
  }
}

export function parseApprovedPairing(input: unknown, expected: PairingRequest): ApprovedPairing {
  try {
    const expectedRequest = parseRequest(expected, expected.worldId);
    const value = record(input);
    exact(value, APPROVED_KEYS);
    const request = parseRequest(requestFields(value), expectedRequest.worldId);
    if (!equalRequest(request, expectedRequest)) invalid();
    if (
      typeof value.principalUid !== 'string' ||
      !value.principalUid.startsWith('office-agent:') ||
      !UUID.test(value.principalUid.slice('office-agent:'.length)) ||
      !matches(value.blockId, UUID) ||
      typeof value.expiresAt !== 'number' ||
      !Number.isSafeInteger(value.expiresAt) ||
      value.expiresAt <= 0 ||
      value.expiresAt > MAX_DATE_MS
    )
      invalid();
    return {
      ...request,
      principalUid: value.principalUid,
      blockId: value.blockId,
      expiresAt: value.expiresAt,
    };
  } catch {
    invalid();
  }
}

export function parseRevokedPairing(input: unknown, pairingId: string): void {
  try {
    if (!matches(pairingId, PAIRING_ID)) invalid();
    const value = record(input);
    exact(value, ['version', 'pairingId', 'revoked']);
    if (value.version !== 1 || value.pairingId !== pairingId || value.revoked !== true) invalid();
  } catch {
    invalid();
  }
}
