import {
  collection,
  doc,
  onSnapshot,
  runTransaction,
  serverTimestamp,
  Timestamp,
} from 'firebase/firestore';
import type { Firestore } from 'firebase/firestore';

import { validWorldId, validWorldName } from './world-contract.js';
import type { World, WorldDraft, WorldPort } from './world-contract.js';

function readWorld(id: string, value: Record<string, unknown>): World {
  if (
    value.version !== 1 ||
    typeof value.name !== 'string' ||
    !validWorldName(value.name) ||
    typeof value.ownerUid !== 'string' ||
    !(value.createdAt instanceof Timestamp)
  ) {
    throw new Error('Unsupported world document.');
  }
  return {
    id,
    name: value.name,
    ownerUid: value.ownerUid,
    createdAtMs: value.createdAt.toMillis(),
  };
}

export function createWorldPort(db: Firestore): WorldPort {
  return {
    draft(name: string): WorldDraft {
      if (!validWorldName(name))
        throw new Error('Use a nonblank name of at most 80 characters without control characters.');
      return { id: doc(collection(db, 'worlds')).id, name };
    },
    async create(draft: WorldDraft, uid: string): Promise<string> {
      if (!validWorldId(draft.id) || !validWorldName(draft.name)) {
        throw new Error('Invalid world draft.');
      }
      const reference = doc(db, 'worlds', draft.id);
      await runTransaction(db, async (transaction) => {
        const snapshot = await transaction.get(reference);
        if (snapshot.exists()) {
          const world = readWorld(snapshot.id, snapshot.data());
          if (world.ownerUid !== uid || world.name !== draft.name)
            throw new Error('World creation conflicts with an existing document.');
          return;
        }
        transaction.set(reference, {
          version: 1,
          name: draft.name,
          ownerUid: uid,
          createdAt: serverTimestamp(),
        });
      });
      return draft.id;
    },
    watch(id: string, changed: (world: World | null) => void, failed: () => void): () => void {
      if (!validWorldId(id)) {
        failed();
        return () => {};
      }
      return onSnapshot(
        doc(db, 'worlds', id),
        { includeMetadataChanges: true },
        (snapshot) => {
          if (snapshot.metadata.fromCache || snapshot.metadata.hasPendingWrites) return;
          try {
            changed(snapshot.exists() ? readWorld(snapshot.id, snapshot.data()) : null);
          } catch {
            failed();
          }
        },
        failed
      );
    },
    watchAdmission(
      uid: string,
      changed: (enabled: boolean) => void,
      failed: () => void
    ): () => void {
      return onSnapshot(
        doc(db, 'testers', uid),
        { includeMetadataChanges: true },
        (snapshot) => {
          if (snapshot.metadata.fromCache || snapshot.metadata.hasPendingWrites) return;
          const data = snapshot.data();
          changed(data?.enabled === true && Object.keys(data).length === 1);
        },
        failed
      );
    },
  };
}
