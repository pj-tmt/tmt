import { digest, requireValue } from '@tmt/colab-client';
import type { HostedAuth } from '../../../../tmt-remote/typescript/remote-client/src/firebase-auth.js';
import type { HostedDeclaration } from '../../../../tmt-remote/typescript/remote-client/src/firebase-declaration.js';
import type { SyncCall } from '../../../../tmt-remote/typescript/remote-client/src/sealed-sync.js';
import {
  HostedBudget,
  hostedBudgetStore,
  type HostedBudgetMath,
  type HostedBudgetStore,
} from './hosted-budget.js';
import type { HostedRunLimit } from './hosted-discovery.js';
import { remoteSdk, responseBytes } from './registration.js';
import { hex } from './export.js';

/** The served Remote bundle owns bootstrap/auth/guard math; Colab never imports Firebase. */
export interface HostedRemoteSdk {
  firebaseRemote: {
    originLink(): Promise<string>;
    connect(link: string, declaration: HostedDeclaration): Promise<HostedAuth>;
  };
  budget: HostedBudgetMath;
}
interface Options {
  load?(): Promise<HostedRemoteSdk>;
  fetch?: typeof fetch;
  store?: HostedBudgetStore;
}
export interface HostedBootstrap {
  auth: HostedAuth;
  declaration: HostedDeclaration;
  budget: HostedBudget;
  limits: ReadonlyMap<string, HostedRunLimit>;
}

/** One release declaration per hosted owner; initialization performs no sign-in or space read. */
export async function bootstrapHosted(
  call: SyncCall,
  options: Options = {},
): Promise<HostedBootstrap> {
  live(call);
  const response = await (options.fetch ?? globalThis.fetch.bind(globalThis))(
    '/tmt/declarations/colab.json',
    {
      redirect: 'error',
      credentials: 'omit',
      cache: 'no-store',
      signal: AbortSignal.any([
        call.signal,
        AbortSignal.timeout(Math.min(10_000, Math.max(1, call.deadlineAtMs - Date.now()))),
      ]),
    },
  );
  const raw = await responseBytes(response, 65536);
  live(call);
  if (!response.ok) throw invalid();
  let bytes: string;
  try {
    bytes = new TextDecoder('utf-8', { fatal: true }).decode(raw);
  } catch {
    throw invalid();
  }
  const declaration = Object.freeze({ bytes, digest: hex(await digest(raw)) });
  live(call);
  // Both entries use registration's single same-origin Remote bundle loader.
  const sdk = options.load
    ? await options.load()
    : ((await remoteSdk()) as unknown as HostedRemoteSdk);
  live(call);
  const link = await sdk.firebaseRemote.originLink();
  live(call);
  const auth = await sdk.firebaseRemote.connect(link, declaration);
  try {
    live(call);
    // connect has verified the grammar and exact digest. These limits are that same release data,
    // not a parallel declaration, persisted fingerprint or caller-selectable provider setting.
    const value = JSON.parse(declaration.bytes) as {
      extension: string;
      resources: { name: string; limits: { maxEntries: number; maxNamespaceBytes: number } }[];
    };
    requireValue(value.extension === 'colab');
    const limits = new Map(
      value.resources.map((resource) => [
        resource.name,
        Object.freeze({
          documents: resource.limits.maxEntries,
          bytes: resource.limits.maxNamespaceBytes,
        }),
      ]),
    );
    return {
      auth,
      declaration,
      limits,
      budget: new HostedBudget(sdk.budget, options.store ?? hostedBudgetStore()),
    };
  } catch (error) {
    await auth.close();
    throw error;
  }
}
function live(call: SyncCall): void {
  if (call.signal.aborted) throw { code: 'REMOTE_STATE_UNAVAILABLE', state: 'cancelled' };
  if (!Number.isFinite(call.deadlineAtMs) || Date.now() >= call.deadlineAtMs)
    throw { code: 'REMOTE_STATE_UNAVAILABLE', state: 'deadline' };
}
const invalid = () => ({ code: 'REMOTE_INPUT_INVALID' as const });
