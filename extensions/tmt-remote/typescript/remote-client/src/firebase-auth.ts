/** One hosted Auth owner; provider-native answers are not local machine-signed receipts. */
import {
  confirmHostedConfig,
  verifyHostedLink,
  HOSTED_BOOTSTRAP_MS,
  type PublicWebConfig,
} from './firebase-bootstrap.js';
import { verifyHostedDeclaration, type HostedDeclaration } from './firebase-declaration.js';
import { ClientError, RefusalError } from './operations.js';

export interface FirebaseAuthPort {
  signInAnonymous(): Promise<string>;
  signOut(): Promise<void>;
  close(): Promise<void>;
}
export interface FirebaseClientModule {
  initialize(config: PublicWebConfig): Promise<FirebaseAuthPort>;
}
export interface HostedAuth {
  signInAnonymous(): Promise<string>;
  signOut(): Promise<void>;
  close(): Promise<void>;
}
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
  try {
    port = await bounded(async (signal) => {
      await verifyHostedDeclaration(declaration);
      const config = await verifyHostedLink(link, deps.origin);
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
  let pending: Promise<string> | undefined;
  let closing: Promise<void> | undefined;
  const live = () => {
    if (closed) throw new RefusalError('REMOTE_CLOSED');
  };
  return {
    signInAnonymous() {
      live();
      if (!pending)
        pending = bounded(async (signal) => {
          try {
            const uid = await port.signInAnonymous();
            live();
            if (signal.aborted)
              throw new ClientError('timeout', 'Firebase authentication check timed out.');
            if (typeof uid !== 'string' || uid.length === 0 || uid.length > 128)
              throw new ClientError(
                'unverifiable_response',
                'Firebase identity could not be confirmed.',
              );
            return uid;
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
      try {
        await bounded(() => port.signOut());
        live();
      } catch (error) {
        throw mapped(error);
      }
    },
    close() {
      closed = true;
      closing ??= port.close().catch((error) => {
        throw mapped(error);
      });
      return closing;
    },
  };
}
