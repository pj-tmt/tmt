/** One hosted Auth owner; provider-native answers are not local machine-signed receipts. */
import {
  confirmHostedConfig,
  verifyHostedLink,
  HOSTED_BOOTSTRAP_MS,
  type PublicWebConfig,
} from './firebase-bootstrap.js';
import { verifyHostedDeclaration, type HostedDeclaration } from './firebase-declaration.js';
import { bindSyncContext, type SealedSyncPort, type SyncAuthPort } from './sealed-sync.js';
import { createSealedSync, type SyncWakePort } from './firebase-sealed-sync.js';
import type { SyncAdmissionGuard } from './firebase-sync-http.js';
export type { SyncAdmissionGuard, SyncAdmissionCost } from './firebase-sync-http.js';
import { ClientError, RefusalError } from './operations.js';

export interface FirebaseAuthPort {
  syncAuth?(uid: string): SyncAuthPort;
  assertSyncIdentity?(uid: string): void;
  wake?: SyncWakePort;
  signInAnonymous(): Promise<string>;
  signOut(): Promise<void>;
  close(): Promise<void>;
}
export interface FirebaseClientModule {
  initialize(config: PublicWebConfig): Promise<FirebaseAuthPort>;
}
export interface HostedAuth {
  sealedSync(space: string, guard: SyncAdmissionGuard): Promise<SealedSyncPort>;
  signInAnonymous(): Promise<string>;
  signOut(): Promise<void>;
  close(): Promise<void>;
}
// All hosted owners in this page share its interest cap.
const hostedPage = { listeners: 0 };
interface Dependencies {
  origin: string;
  fetch: typeof fetch;
  load(): Promise<FirebaseClientModule>;
}
function mapped(error: unknown): Error {
  if (error instanceof ClientError || error instanceof RefusalError) return error;
  const code =
    typeof error === 'object' && error !== null && 'code' in error ? error.code : undefined;
  if (code === 'remote/auth-pending') return new RefusalError('REMOTE_STATE_UNAVAILABLE');
  if (code === 'auth/operation-not-allowed') return new RefusalError('REMOTE_SCOPE_DENIED');
  if (code === 'auth/too-many-requests') return new RefusalError('REMOTE_RATE_LIMITED');
  return new ClientError('transport_failure', 'Firebase authentication could not be confirmed.');
}
async function bounded<T>(action: (signal: AbortSignal) => Promise<T>): Promise<T> {
  const controller = new AbortController();
  let timer: ReturnType<typeof setTimeout> | undefined;
  const deadline = new Promise<never>((_, reject) => {
    timer = setTimeout(() => {
      controller.abort();
      reject(new ClientError('timeout', 'Firebase authentication check timed out.'));
    }, HOSTED_BOOTSTRAP_MS);
  });
  try {
    return await Promise.race([action(controller.signal), deadline]);
  } finally {
    clearTimeout(timer);
  }
}

/** Ports are internal test edges; the browser entry supplies same-origin fetch and lazy import. */
export async function connectHosted(
  link: string,
  declaration: HostedDeclaration,
  deps: Dependencies,
): Promise<HostedAuth> {
  let port: FirebaseAuthPort;
  let verifiedConfig!: PublicWebConfig;
  const frozenDeclaration = { ...declaration };
  try {
    port = await bounded(async (signal) => {
      await verifyHostedDeclaration(frozenDeclaration);
      const config = await verifyHostedLink(link, deps.origin);
      verifiedConfig = config;
      await confirmHostedConfig(config, deps.origin, deps.fetch, signal);
      if (signal.aborted) throw new ClientError('timeout', 'Hosted bootstrap timed out.');
      const module = await deps.load().catch(() => {
        throw new RefusalError('REMOTE_STATE_UNAVAILABLE');
      });
      if (signal.aborted) throw new ClientError('timeout', 'Hosted bootstrap timed out.');
      const initialized = await module.initialize(config);
      if (signal.aborted) {
        await initialized.close();
        throw new ClientError('timeout', 'Hosted bootstrap timed out.');
      }
      return initialized;
    });
  } catch (error) {
    throw mapped(error);
  }
  let closed = false;
  let uid: string | undefined;
  let generation = 0;
  const syncOwners = new Set<SealedSyncPort>();
  let pending: Promise<string> | undefined;
  let closing: Promise<void> | undefined;
  const live = () => {
    if (closed) throw new RefusalError('REMOTE_CLOSED');
  };
  return Object.freeze({
    async sealedSync(space: string, guard: SyncAdmissionGuard) {
      live();
      if (!uid || !port.syncAuth || !port.wake) throw new RefusalError('REMOTE_STATE_UNAVAILABLE');
      if (!guard || typeof guard.admit !== 'function')
        throw new RefusalError('REMOTE_INPUT_INVALID');
      const identity = uid,
        stamp = generation;
      const fence = () => {
        live();
        if (stamp !== generation || uid !== identity)
          throw new RefusalError('REMOTE_STATE_UNAVAILABLE');
        port.assertSyncIdentity?.(identity);
      };
      const context = await bindSyncContext(
        verifiedConfig,
        frozenDeclaration,
        space,
        port.syncAuth(identity),
      );
      fence();
      const bound = createSealedSync(context, guard, deps.fetch, port.wake, fence, hostedPage, () =>
        syncOwners.delete(bound),
      );
      syncOwners.add(bound);
      return bound;
    },
    signInAnonymous() {
      live();
      if (!pending)
        pending = bounded(async (signal) => {
          try {
            const signedIn = await port.signInAnonymous();
            live();
            if (signal.aborted)
              throw new ClientError('timeout', 'Firebase authentication check timed out.');
            if (typeof signedIn !== 'string' || signedIn.length === 0 || signedIn.length > 128)
              throw new ClientError(
                'unverifiable_response',
                'Firebase identity could not be confirmed.',
              );
            uid = signedIn;
            return signedIn;
          } catch (error) {
            throw mapped(error);
          }
        }).finally(() => {
          pending = undefined;
        });
      return pending;
    },
    async signOut() {
      live();
      if (pending) throw new RefusalError('REMOTE_STATE_UNAVAILABLE');
      generation++;
      uid = undefined;
      for (const owner of syncOwners) owner.close();
      syncOwners.clear();
      try {
        await bounded(() => port.signOut());
        live();
      } catch (error) {
        throw mapped(error);
      }
    },
    close() {
      closed = true;
      generation++;
      uid = undefined;
      for (const owner of syncOwners) owner.close();
      syncOwners.clear();
      closing ??= port.close().catch((error) => {
        throw mapped(error);
      });
      return closing;
    },
  });
}
