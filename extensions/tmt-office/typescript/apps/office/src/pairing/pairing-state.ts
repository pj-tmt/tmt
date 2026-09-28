import { PairingActionError, snapshotPairingRequest } from './pairing-contract.js';
import type {
  ApprovedPairing,
  PairingRequest,
  PairingPort,
  PairingReplacement,
} from './pairing-contract.js';

interface PairingSnapshot {
  busy: boolean;
  attempted: boolean;
  revocationAttempted: boolean;
  approved: ApprovedPairing | null;
  revoked: boolean;
  error: string | null;
  replacement: PairingReplacement | null;
}

/** One mounted request owns actions; disposal fences results, not remote writes. */
export function createPairingState(port: PairingPort, request: PairingRequest, ownerUid: string) {
  const originalRequest = snapshotPairingRequest(request);
  let snapshot: PairingSnapshot = {
    busy: false,
    attempted: false,
    revocationAttempted: false,
    approved: null,
    revoked: false,
    error: null,
    replacement: null,
  };
  let disposed = false;
  const listeners = new Set<() => void>();
  function publish(change: Partial<PairingSnapshot>) {
    if (disposed) return;
    snapshot = { ...snapshot, ...change };
    for (const listener of listeners) listener();
  }
  async function act(action: () => Promise<Partial<PairingSnapshot>>, kind: 'approve' | 'revoke') {
    if (
      disposed ||
      snapshot.busy ||
      snapshot.revoked ||
      (kind === 'approve' && snapshot.revocationAttempted)
    )
      return;
    publish({
      busy: true,
      error: null,
      ...(kind === 'approve' ? { attempted: true } : { revocationAttempted: true }),
    });
    try {
      publish(await action());
    } catch (error) {
      publish({
        error:
          error instanceof PairingActionError
            ? error.message
            : new PairingActionError('uncertain').message,
      });
    } finally {
      publish({ busy: false });
    }
  }
  return {
    getSnapshot: () => snapshot,
    subscribe(listener: () => void) {
      if (disposed) return () => {};
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    selectReplacement(replacement: PairingReplacement | null) {
      if (disposed || snapshot.attempted || snapshot.revocationAttempted) return;
      publish({ replacement: replacement ? { ...replacement } : null });
    },
    approve: () =>
      act(
        async () => ({
          approved: snapshot.replacement
            ? await port.approve(originalRequest, ownerUid, snapshot.replacement)
            : await port.approve(originalRequest, ownerUid),
        }),
        'approve'
      ),
    revoke: () =>
      act(async () => {
        await port.revoke(originalRequest, ownerUid);
        return { approved: null, revoked: true };
      }, 'revoke'),
    dispose() {
      disposed = true;
      listeners.clear();
      snapshot = {
        busy: false,
        attempted: false,
        revocationAttempted: false,
        approved: null,
        revoked: false,
        error: null,
        replacement: null,
      };
    },
  };
}

export type PairingState = ReturnType<typeof createPairingState>;
