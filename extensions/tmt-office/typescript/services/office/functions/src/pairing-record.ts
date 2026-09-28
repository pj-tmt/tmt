import { Timestamp } from 'firebase-admin/firestore';
import {
  APPROVAL_MS,
  GRANT_MS,
  PAIRING_ID,
  PRINCIPAL_UID,
  UUID,
  PairingError,
  exact,
  isTimestampMillis,
  object,
  parseApproval,
} from './pairing-contract.js';
import type { Approval } from './pairing-contract.js';

export interface Pairing {
  request: Approval;
  ownerUid: string;
  principalUid: string;
  blockId: string;
  createdAt: Timestamp;
  expiresAt: Timestamp;
  enabled: boolean;
  claimed: boolean;
  nextClaimAt: Timestamp;
  replacesPrincipalUid?: string;
}

export interface Grant {
  version: 1;
  ownerUid: string;
  installationId: string;
  identityId: string;
  blockId: string;
  capabilities: Approval['capabilities'];
  enabled: boolean;
  createdAt: Timestamp;
  expiresAt: Timestamp;
  replacedByPairingId?: string;
}

function unavailable(): never {
  throw new PairingError('PAIRING_UNAVAILABLE');
}

export function timestampMillis(value: Timestamp): number {
  if (value.nanoseconds % 1_000_000 !== 0) unavailable();
  const millis = value.toMillis();
  if (!isTimestampMillis(millis)) unavailable();
  return millis;
}

export function decodePairing(input: unknown, id: string): Pairing {
  try {
    const value = object(input);
    exact(value, [
      'request',
      'ownerUid',
      'principalUid',
      'blockId',
      'createdAt',
      'expiresAt',
      'enabled',
      'claimed',
      'nextClaimAt',
      ...(Object.hasOwn(value, 'replacesPrincipalUid') ? ['replacesPrincipalUid'] : []),
    ]);
    const request = parseApproval(value.request);
    if (
      request.pairingId !== id ||
      typeof value.ownerUid !== 'string' ||
      !value.ownerUid ||
      typeof value.principalUid !== 'string' ||
      !PRINCIPAL_UID.test(value.principalUid) ||
      typeof value.blockId !== 'string' ||
      !UUID.test(value.blockId) ||
      !(value.createdAt instanceof Timestamp) ||
      !(value.expiresAt instanceof Timestamp) ||
      !(value.nextClaimAt instanceof Timestamp) ||
      typeof value.enabled !== 'boolean' ||
      typeof value.claimed !== 'boolean' ||
      (Object.hasOwn(value, 'replacesPrincipalUid') &&
        (typeof value.replacesPrincipalUid !== 'string' ||
          !PRINCIPAL_UID.test(value.replacesPrincipalUid)))
    )
      return unavailable();
    const replacesPrincipalUid = value.replacesPrincipalUid;
    return {
      request,
      ownerUid: value.ownerUid,
      principalUid: value.principalUid,
      blockId: value.blockId,
      createdAt: value.createdAt,
      expiresAt: value.expiresAt,
      enabled: value.enabled,
      claimed: value.claimed,
      nextClaimAt: value.nextClaimAt,
      ...(typeof replacesPrincipalUid === 'string' ? { replacesPrincipalUid } : {}),
    };
  } catch {
    return unavailable();
  }
}

export function decodeGrant(input: unknown): Grant {
  try {
    const value = object(input);
    exact(value, [
      'version',
      'ownerUid',
      'installationId',
      'identityId',
      'blockId',
      'capabilities',
      'enabled',
      'createdAt',
      'expiresAt',
      ...(Object.hasOwn(value, 'replacedByPairingId') ? ['replacedByPairingId'] : []),
    ]);
    const createdAt = value.createdAt instanceof Timestamp ? value.createdAt : unavailable();
    const expiresAt = value.expiresAt instanceof Timestamp ? value.expiresAt : unavailable();
    const capabilities = value.capabilities;
    if (
      value.version !== 1 ||
      typeof value.ownerUid !== 'string' ||
      !value.ownerUid ||
      typeof value.installationId !== 'string' ||
      !UUID.test(value.installationId) ||
      typeof value.identityId !== 'string' ||
      !UUID.test(value.identityId) ||
      typeof value.blockId !== 'string' ||
      !UUID.test(value.blockId) ||
      !Array.isArray(capabilities) ||
      (capabilities.length !== 1 && capabilities.length !== 2) ||
      capabilities[0] !== 'layout.read' ||
      (capabilities.length === 2 && capabilities[1] !== 'layout.write') ||
      typeof value.enabled !== 'boolean' ||
      (Object.hasOwn(value, 'replacedByPairingId') &&
        (typeof value.replacedByPairingId !== 'string' ||
          !PAIRING_ID.test(value.replacedByPairingId)))
    )
      return unavailable();
    const replacedByPairingId = value.replacedByPairingId;
    const grant: Grant = {
      version: 1,
      ownerUid: value.ownerUid,
      installationId: value.installationId,
      identityId: value.identityId,
      blockId: value.blockId,
      capabilities: capabilities.length === 1 ? ['layout.read'] : ['layout.read', 'layout.write'],
      enabled: value.enabled,
      createdAt,
      expiresAt,
      ...(typeof replacedByPairingId === 'string' ? { replacedByPairingId } : {}),
    };
    if (grant.enabled && grant.replacedByPairingId !== undefined) return unavailable();
    timestampMillis(grant.createdAt);
    timestampMillis(grant.expiresAt);
    return grant;
  } catch {
    return unavailable();
  }
}

export function validApprovalTimestamps(pairing: Pairing): void {
  const createdAt = timestampMillis(pairing.createdAt);
  const expiresAt = timestampMillis(pairing.expiresAt);
  timestampMillis(pairing.nextClaimAt);
  if (expiresAt - createdAt !== APPROVAL_MS) unavailable();
}

export function validGrantDuration(grant: Grant): void {
  const createdAt = timestampMillis(grant.createdAt);
  const expiresAt = timestampMillis(grant.expiresAt);
  if (expiresAt <= createdAt || expiresAt - createdAt > GRANT_MS) unavailable();
}
