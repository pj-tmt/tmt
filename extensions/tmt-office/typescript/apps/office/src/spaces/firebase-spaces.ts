import {
  collection,
  documentId,
  getDocsFromServer,
  limit,
  orderBy,
  query,
  startAfter,
  Timestamp,
} from 'firebase/firestore';
import type { Firestore } from 'firebase/firestore';
import { validWorldId } from '../worlds/world-contract.js';
import { createBlockPort } from '../blocks/firebase-blocks.js';
import { SPACE_PAGE_SIZE } from './space-contract.js';
import type { AgentSpace, SpacePort } from './space-contract.js';

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
function validPrincipal(value: string): boolean {
  return value.startsWith('office-agent:') && UUID.test(value.slice('office-agent:'.length));
}

/** A read projection, not a second authority model. Rules decide who may read. */
export function readAgentSpace(
  principalUid: string,
  value: Record<string, unknown>,
  ownerUid: string
): AgentSpace {
  const capabilities = value.capabilities;
  const hasReplacement = Object.hasOwn(value, 'replacedByPairingId');
  if (
    !validPrincipal(principalUid) ||
    Object.keys(value).length !== (hasReplacement ? 10 : 9) ||
    (hasReplacement &&
      (value.enabled !== false ||
        typeof value.replacedByPairingId !== 'string' ||
        !/^[0-9a-f]{64}$/.test(value.replacedByPairingId))) ||
    value.version !== 1 ||
    value.ownerUid !== ownerUid ||
    !['installationId', 'identityId', 'blockId'].every(
      (key) => typeof value[key] === 'string' && UUID.test(value[key])
    ) ||
    typeof value.enabled !== 'boolean' ||
    !(value.createdAt instanceof Timestamp) ||
    !(value.expiresAt instanceof Timestamp) ||
    value.createdAt.toMillis() <= 0 ||
    value.expiresAt.toMillis() <= value.createdAt.toMillis() ||
    value.expiresAt.toMillis() - value.createdAt.toMillis() > 86_400_000 ||
    !Array.isArray(capabilities) ||
    capabilities[0] !== 'layout.read' ||
    !(
      capabilities.length === 1 ||
      (capabilities.length === 2 && capabilities[1] === 'layout.write')
    )
  )
    throw new Error('Unsupported agent grant.');
  return {
    principalUid,
    installationId: value.installationId as string,
    identityId: value.identityId as string,
    blockId: value.blockId as string,
    capabilities: capabilities.length === 1 ? ['layout.read'] : ['layout.read', 'layout.write'],
    enabled: value.enabled,
    expiresAtMs: value.expiresAt.toMillis(),
    ...(hasReplacement ? { replacedByPairingId: value.replacedByPairingId as string } : {}),
  };
}

export function createSpacePort(db: Firestore): SpacePort {
  return {
    async list(worldId, ownerUid, after) {
      if (!validWorldId(worldId) || !ownerUid || (after !== undefined && !validPrincipal(after)))
        throw new Error('Invalid space query.');
      const snapshot = await getDocsFromServer(
        query(
          collection(db, 'worlds', worldId, 'agentGrants'),
          orderBy(documentId()),
          ...(after === undefined ? [] : [startAfter(after)]),
          limit(SPACE_PAGE_SIZE)
        )
      );
      const entries = snapshot.docs.map((row) => readAgentSpace(row.id, row.data(), ownerUid));
      return {
        entries,
        next: entries.length === SPACE_PAGE_SIZE ? entries.at(-1)!.principalUid : null,
      };
    },
    block: (blockId) => createBlockPort(db, blockId),
  };
}
