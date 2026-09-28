import { randomUUID } from 'node:crypto';
import { Timestamp } from 'firebase-admin/firestore';
import type { Firestore, Transaction } from 'firebase-admin/firestore';
import {
  APPROVAL_MS,
  CLAIM_INTERVAL_MS,
  GRANT_MS,
  isTimestampMillis,
  MAX_TIMESTAMP_MS,
  PRINCIPAL_UID,
  RENEWAL_WINDOW_MS,
  PairingError,
} from './pairing-contract.js';
import type { Approval } from './pairing-contract.js';
import {
  decodeGrant,
  decodePairing,
  timestampMillis,
  validApprovalTimestamps,
  validGrantDuration,
} from './pairing-record.js';
import type { Grant, Pairing } from './pairing-record.js';

export interface ApprovedBinding extends Approval {
  principalUid: string;
  blockId: string;
  expiresAt: number;
}

export interface ClaimedBinding extends ApprovedBinding {
  grantExpiresAt: number;
}

export interface AgentIdentity {
  uid: string;
  installationId: string;
  identityId: string;
}

export type RevocationActor =
  | { kind: 'owner'; uid: string }
  | { kind: 'agent'; agent: AgentIdentity }
  | { kind: 'proof' }
  | { kind: 'ownerApproval'; uid: string; request: Approval };

function matchesAgent(pairing: Pairing, agent: AgentIdentity): boolean {
  return (
    pairing.principalUid === agent.uid &&
    pairing.request.installationId === agent.installationId &&
    pairing.request.identityId === agent.identityId
  );
}

export interface RenewedBinding {
  version: 1;
  pairingId: string;
  worldId: string;
  installationId: string;
  identityId: string;
  principalUid: string;
  blockId: string;
  capabilities: Approval['capabilities'];
  grantExpiresAt: number;
}

function unavailable(): never {
  throw new PairingError('PAIRING_UNAVAILABLE');
}

function validApproval(pairing: Pairing, now: number, temporal: 'active' | 'renewal'): void {
  const createdAt = timestampMillis(pairing.createdAt);
  const expiresAt = timestampMillis(pairing.expiresAt);
  validApprovalTimestamps(pairing);
  if (!pairing.enabled || createdAt > now || (temporal === 'active' && expiresAt <= now))
    unavailable();
}

function view(pairing: Pairing): ApprovedBinding {
  return {
    ...pairing.request,
    principalUid: pairing.principalUid,
    blockId: pairing.blockId,
    expiresAt: pairing.expiresAt.toMillis(),
  };
}

function sameRequest(left: Approval, right: Approval): boolean {
  return JSON.stringify(left) === JSON.stringify(right);
}

function validGrant(
  input: unknown,
  pairing: Pairing,
  now: number,
  temporal: 'active' | 'renewal' = 'active'
): number {
  const value = decodeGrant(input);
  validGrantDuration(value);
  const createdAt = timestampMillis(value.createdAt);
  const expiresAt = timestampMillis(value.expiresAt);
  if (
    value.ownerUid !== pairing.ownerUid ||
    value.installationId !== pairing.request.installationId ||
    value.identityId !== pairing.request.identityId ||
    value.blockId !== pairing.blockId ||
    JSON.stringify(value.capabilities) !== JSON.stringify(pairing.request.capabilities) ||
    value.enabled !== true ||
    createdAt > now ||
    (temporal === 'active' && expiresAt <= now)
  )
    unavailable();
  return expiresAt;
}

function replacementConflict(): never {
  throw new PairingError('PAIRING_CONFLICT');
}

function sourceGrant(input: unknown, ownerUid: string, now: number): Grant {
  try {
    const grant = decodeGrant(input);
    validGrantDuration(grant);
    const createdAt = timestampMillis(grant.createdAt);
    if (grant.ownerUid !== ownerUid || grant.enabled !== false || createdAt > now)
      return replacementConflict();
    return grant;
  } catch {
    return replacementConflict();
  }
}

function validPredecessor(
  input: unknown,
  id: string,
  source: Grant,
  sourcePrincipalUid: string,
  ownerUid: string,
  request: Approval,
  now: number
): Pairing {
  try {
    const predecessor = decodePairing(input, id);
    const createdAt = timestampMillis(predecessor.createdAt);
    const expiresAt = timestampMillis(predecessor.expiresAt);
    validApprovalTimestamps(predecessor);
    if (
      predecessor.ownerUid !== ownerUid ||
      predecessor.request.worldId !== request.worldId ||
      predecessor.blockId !== source.blockId ||
      predecessor.replacesPrincipalUid !== sourcePrincipalUid ||
      predecessor.claimed ||
      createdAt > now ||
      (predecessor.enabled && expiresAt > now)
    )
      return replacementConflict();
    return predecessor;
  } catch {
    return replacementConflict();
  }
}

function validSourceReservation(input: unknown, pairing: Pairing, now: number): void {
  if (pairing.replacesPrincipalUid === undefined) return;
  try {
    const source = sourceGrant(input, pairing.ownerUid, now);
    if (
      source.blockId !== pairing.blockId ||
      source.replacedByPairingId !== pairing.request.pairingId
    )
      unavailable();
  } catch {
    unavailable();
  }
}

function renewed(pairing: Pairing, grantExpiresAt: number): RenewedBinding {
  return {
    version: 1,
    pairingId: pairing.request.pairingId,
    worldId: pairing.request.worldId,
    installationId: pairing.request.installationId,
    identityId: pairing.request.identityId,
    principalUid: pairing.principalUid,
    blockId: pairing.blockId,
    capabilities: pairing.request.capabilities,
    grantExpiresAt,
  };
}

export function createPairingStore(db: Firestore, clock: () => number = Date.now) {
  const pairingRef = (id: string) => db.collection('officePairings').doc(id);
  const grantRefFor = (worldId: string, principalUid: string) =>
    db.collection('worlds').doc(worldId).collection('agentGrants').doc(principalUid);
  const grantRef = (pairing: Pairing) => grantRefFor(pairing.request.worldId, pairing.principalUid);

  async function ownsWorld(tx: Transaction, uid: string, worldId: string): Promise<boolean> {
    const [tester, world] = await tx.getAll(
      db.collection('testers').doc(uid),
      db.collection('worlds').doc(worldId)
    );
    const admission = tester.data();
    return (
      admission?.enabled === true &&
      Object.keys(admission).length === 1 &&
      world.data()?.ownerUid === uid
    );
  }

  async function current(tx: Transaction, id: string, now: number): Promise<Pairing> {
    const pairing = decodePairing((await tx.get(pairingRef(id))).data(), id);
    validApproval(pairing, now, 'active');
    if (!(await ownsWorld(tx, pairing.ownerUid, pairing.request.worldId))) unavailable();
    if (pairing.replacesPrincipalUid !== undefined) {
      const source = await tx.get(
        grantRefFor(pairing.request.worldId, pairing.replacesPrincipalUid)
      );
      validSourceReservation(source.data(), pairing, now);
    }
    return pairing;
  }

  return {
    async approve(
      uid: string,
      request: Approval,
      replacesPrincipalUid?: string
    ): Promise<ApprovedBinding> {
      if (replacesPrincipalUid !== undefined && !PRINCIPAL_UID.test(replacesPrincipalUid))
        throw new PairingError('INVALID_ARGUMENT');
      // Random IDs are stable across transaction retries; no minting or delivery in tx.
      const principalUid = `office-agent:${randomUUID()}`;
      const freshBlockId = randomUUID();
      return db.runTransaction(async (tx) => {
        const now = clock();
        if (!isTimestampMillis(now) || now > MAX_TIMESTAMP_MS - APPROVAL_MS) unavailable();
        if (!(await ownsWorld(tx, uid, request.worldId)))
          throw new PairingError('PERMISSION_DENIED');
        const existing = await tx.get(pairingRef(request.pairingId));
        const sourceRef =
          replacesPrincipalUid === undefined
            ? undefined
            : grantRefFor(request.worldId, replacesPrincipalUid);
        const sourceSnapshot = sourceRef === undefined ? undefined : await tx.get(sourceRef);
        const source =
          replacesPrincipalUid === undefined
            ? undefined
            : sourceGrant(sourceSnapshot?.data(), uid, now);
        const reservedPairingId = source?.replacedByPairingId;
        const predecessorSnapshot =
          reservedPairingId !== undefined && reservedPairingId !== request.pairingId
            ? await tx.get(pairingRef(reservedPairingId))
            : undefined;
        const predecessor =
          source !== undefined &&
          reservedPairingId !== undefined &&
          reservedPairingId !== request.pairingId
            ? validPredecessor(
                predecessorSnapshot?.data(),
                reservedPairingId,
                source,
                replacesPrincipalUid!,
                uid,
                request,
                now
              )
            : undefined;
        const predecessorGrantSnapshot =
          predecessor !== undefined
            ? await tx.get(grantRefFor(request.worldId, predecessor.principalUid))
            : undefined;
        if (existing.exists) {
          const pairing = decodePairing(existing.data(), request.pairingId);
          if (
            pairing.ownerUid !== uid ||
            !sameRequest(pairing.request, request) ||
            pairing.replacesPrincipalUid !== replacesPrincipalUid
          )
            throw new PairingError('PAIRING_CONFLICT');
          validApproval(pairing, now, 'active');
          if (replacesPrincipalUid !== undefined) {
            if (
              source === undefined ||
              source.blockId !== pairing.blockId ||
              reservedPairingId !== request.pairingId
            )
              return replacementConflict();
          }
          if (pairing.claimed) validGrant((await tx.get(grantRef(pairing))).data(), pairing, now);
          return view(pairing);
        }
        if (source !== undefined && reservedPairingId !== undefined) {
          if (reservedPairingId === request.pairingId) return replacementConflict();
          if (predecessor === undefined) return replacementConflict();
          if (predecessorGrantSnapshot?.exists) return replacementConflict();
          tx.update(pairingRef(predecessor.request.pairingId), { enabled: false });
        }
        const pairing: Pairing = {
          request,
          ownerUid: uid,
          principalUid,
          blockId: source?.blockId ?? freshBlockId,
          createdAt: Timestamp.fromMillis(now),
          expiresAt: Timestamp.fromMillis(now + APPROVAL_MS),
          enabled: true,
          claimed: false,
          nextClaimAt: Timestamp.fromMillis(now),
          ...(replacesPrincipalUid !== undefined ? { replacesPrincipalUid } : {}),
        };
        if (sourceRef !== undefined)
          tx.update(sourceRef, { replacedByPairingId: request.pairingId });
        tx.create(pairingRef(request.pairingId), pairing);
        return view(pairing);
      });
    },

    async reserveClaim(id: string): Promise<ClaimedBinding> {
      return db.runTransaction(async (tx) => {
        const now = clock();
        const pairing = await current(tx, id, now);
        if (pairing.nextClaimAt.toMillis() > now) throw new PairingError('RETRY_LATER');
        const grant = await tx.get(grantRef(pairing));
        const grantExpiresAt = pairing.claimed
          ? validGrant(grant.data(), pairing, now)
          : now + GRANT_MS;
        if (!pairing.claimed) {
          if (grant.exists) unavailable();
          tx.create(grantRef(pairing), {
            version: 1,
            ownerUid: pairing.ownerUid,
            installationId: pairing.request.installationId,
            identityId: pairing.request.identityId,
            blockId: pairing.blockId,
            capabilities: pairing.request.capabilities,
            enabled: true,
            createdAt: Timestamp.fromMillis(now),
            expiresAt: Timestamp.fromMillis(grantExpiresAt),
          });
        }
        tx.update(pairingRef(id), {
          claimed: true,
          nextClaimAt: Timestamp.fromMillis(now + CLAIM_INTERVAL_MS),
        });
        return { ...view(pairing), grantExpiresAt };
      });
    },

    async confirmClaim(id: string): Promise<void> {
      await db.runTransaction(async (tx) => {
        const now = clock();
        const pairing = await current(tx, id, now);
        if (!pairing.claimed) unavailable();
        validGrant((await tx.get(grantRef(pairing))).data(), pairing, now);
      });
    },

    async renew(
      agent: AgentIdentity,
      id: string,
      expectedGrantExpiresAt: number
    ): Promise<RenewedBinding> {
      return db.runTransaction(async (tx) => {
        const now = clock();
        if (!isTimestampMillis(now) || !isTimestampMillis(expectedGrantExpiresAt)) unavailable();
        const pairing = decodePairing((await tx.get(pairingRef(id))).data(), id);
        validApproval(pairing, now, 'renewal');
        if (!pairing.claimed || !(await ownsWorld(tx, pairing.ownerUid, pairing.request.worldId)))
          unavailable();
        if (!matchesAgent(pairing, agent)) unavailable();

        const grant = await tx.get(grantRef(pairing));
        const currentGrantExpiresAt = validGrant(grant.data(), pairing, now, 'renewal');
        if (expectedGrantExpiresAt > currentGrantExpiresAt)
          throw new PairingError('PAIRING_CONFLICT');
        const withinRenewalWindow =
          currentGrantExpiresAt <= now || currentGrantExpiresAt - now <= RENEWAL_WINDOW_MS;
        if (expectedGrantExpiresAt < currentGrantExpiresAt || !withinRenewalWindow)
          return renewed(pairing, currentGrantExpiresAt);

        if (now > MAX_TIMESTAMP_MS - GRANT_MS) unavailable();
        const grantExpiresAt = now + GRANT_MS;
        tx.update(grant.ref, {
          createdAt: Timestamp.fromMillis(now),
          expiresAt: Timestamp.fromMillis(grantExpiresAt),
        });
        return renewed(pairing, grantExpiresAt);
      });
    },

    async revoke(actor: RevocationActor, id: string): Promise<void> {
      const tombstonePrincipalUid =
        actor.kind === 'ownerApproval' ? `office-agent:${randomUUID()}` : undefined;
      const tombstoneBlockId = actor.kind === 'ownerApproval' ? randomUUID() : undefined;
      await db.runTransaction(async (tx) => {
        const now = clock();
        const snapshot = await tx.get(pairingRef(id));
        if (actor.kind === 'ownerApproval' && !snapshot.exists) {
          if (
            actor.request.pairingId !== id ||
            !isTimestampMillis(now) ||
            now > MAX_TIMESTAMP_MS - APPROVAL_MS
          )
            unavailable();
          if (!(await ownsWorld(tx, actor.uid, actor.request.worldId)))
            throw new PairingError('PERMISSION_DENIED');
          tx.create(pairingRef(id), {
            request: actor.request,
            ownerUid: actor.uid,
            principalUid: tombstonePrincipalUid!,
            blockId: tombstoneBlockId!,
            createdAt: Timestamp.fromMillis(now),
            expiresAt: Timestamp.fromMillis(now + APPROVAL_MS),
            enabled: false,
            claimed: false,
            nextClaimAt: Timestamp.fromMillis(now),
          });
          return;
        }
        let pairing: Pairing;
        try {
          pairing = decodePairing(snapshot.data(), id);
        } catch {
          throw new PairingError(
            actor.kind === 'owner' || actor.kind === 'ownerApproval'
              ? 'PERMISSION_DENIED'
              : 'PAIRING_UNAVAILABLE'
          );
        }
        if (actor.kind === 'owner' || actor.kind === 'ownerApproval') {
          if (
            pairing.ownerUid !== actor.uid ||
            !(await ownsWorld(tx, actor.uid, pairing.request.worldId))
          )
            throw new PairingError('PERMISSION_DENIED');
          if (actor.kind === 'ownerApproval' && !sameRequest(pairing.request, actor.request))
            throw new PairingError('PAIRING_CONFLICT');
        } else if (
          actor.kind === 'agent' &&
          (!pairing.claimed || !matchesAgent(pairing, actor.agent))
        ) {
          unavailable();
        }
        const grant = await tx.get(grantRef(pairing));
        tx.update(pairingRef(id), { enabled: false });
        if (grant.exists) tx.update(grantRef(pairing), { enabled: false });
      });
    },
  };
}

export type PairingStore = ReturnType<typeof createPairingStore>;
