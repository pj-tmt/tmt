import { doc, onSnapshot, runTransaction, serverTimestamp, Timestamp } from 'firebase/firestore';
import type { Firestore } from 'firebase/firestore';
import { FirebaseError } from 'firebase/app';
import { validWorldId } from '../worlds/world-contract.js';
import {
  BlockConflict,
  sameLayout,
  validLayout,
  encodeLayout,
  decodeLayout,
} from './block-contract.js';
import type { Block, BlockPort } from './block-contract.js';

function readBlock(value: Record<string, unknown>): Block {
  if (
    Object.keys(value).length !== 4 ||
    value.version !== 1 ||
    !Number.isSafeInteger(value.revision) ||
    (value.revision as number) < 1 ||
    !(value.updatedAt instanceof Timestamp)
  ) {
    throw new Error('Unsupported block document.');
  }
  return {
    revision: value.revision as number,
    objects: decodeLayout(value.objects),
    updatedAtMs: value.updatedAt.toMillis(),
  };
}
export function createBlockPort(db: Firestore, blockId = 'home'): BlockPort {
  if (!/^(home|[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})$/.test(blockId))
    throw new Error('Invalid block ID.');
  function reference(worldId: string) {
    if (!validWorldId(worldId)) throw new Error('Invalid world ID.');
    return doc(db, 'worlds', worldId, 'blocks', blockId);
  }
  return {
    watch(worldId, changed, failed) {
      return onSnapshot(
        reference(worldId),
        { includeMetadataChanges: true },
        (snapshot) => {
          if (snapshot.metadata.fromCache || snapshot.metadata.hasPendingWrites) return;
          try {
            changed(snapshot.exists() ? readBlock(snapshot.data()) : null);
          } catch {
            failed();
          }
        },
        failed
      );
    },
    async apply(worldId, revision, objects) {
      if (
        !Number.isSafeInteger(revision) ||
        revision < 0 ||
        revision >= Number.MAX_SAFE_INTEGER ||
        !validLayout(objects)
      ) {
        throw new Error('Invalid block edit.');
      }
      const target = reference(worldId);
      // One-shot confirmation must not depend on the watch channel reconnecting.
      const readCurrent = () => runTransaction(db, (transaction) => transaction.get(target));
      try {
        await runTransaction(db, async (transaction) => {
          const snapshot = await transaction.get(target);
          const current = snapshot.exists() ? readBlock(snapshot.data()) : null;
          if (current?.revision === revision + 1 && sameLayout(current.objects, objects)) return;
          if ((current?.revision ?? 0) !== revision) throw new BlockConflict();
          transaction.set(target, {
            version: 1,
            revision: revision + 1,
            objects: encodeLayout(objects),
            updatedAt: serverTimestamp(),
          });
        });
      } catch (error) {
        // Rules may reject a stale revision before Firestore retries a racing
        // transaction. Classify it only after a fresh, authorized server read;
        // never turn a revoked/foreign user's denial into a conflict or success.
        if (error instanceof FirebaseError && error.code === 'permission-denied') {
          const observed = await readCurrent().catch(() => {
            throw error;
          });
          if (observed.exists()) {
            const current = readBlock(observed.data());
            if (current.revision === revision + 1 && sameLayout(current.objects, objects))
              return current;
            if (current.revision !== revision) throw new BlockConflict();
          }
        }
        throw error;
      }
      // A separate server read confirms canonical timestamps without relying on
      // pending-write snapshots. It may observe a newer edit, which must be kept.
      const saved = await readCurrent();
      if (!saved.exists()) throw new Error('Block unavailable.');
      return readBlock(saved.data());
    },
  };
}
