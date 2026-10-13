/** Qualifying emulator entry only: ordinary browser Auth + product transport, no Admin SDK. */
import { getApp } from 'firebase/app';
import { connectAuthEmulator, getAuth } from 'firebase/auth';
import { connectFirestoreEmulator, getFirestore } from 'firebase/firestore';
import { initialize } from '../../src/firebase-client.js';
import { connectHosted } from '../../src/firebase-auth.js';
import type { PublicWebConfig } from '../../src/firebase-bootstrap.js';
import type { HostedDeclaration } from '../../src/firebase-declaration.js';
import type {
  SealedSyncPort,
  SyncCall,
  SyncInterest,
  SyncMutation,
  SyncQueryRequest,
} from '../../src/sealed-sync.js';

export async function start(
  config: PublicWebConfig,
  declaration: HostedDeclaration,
  link: string,
  endpoints: { authPort: number; firestorePort: number },
) {
  if (!config.projectId.startsWith('demo-')) throw Error('emulator project required');
  let sync: SealedSyncPort | undefined;
  const costs: unknown[] = [];
  const auth = await connectHosted(link, declaration, {
    origin: new URL(link).origin,
    fetch: (async (input: string | URL | Request, options?: RequestInit) => {
      const url = new URL(String(input));
      if (url.origin === new URL(link).origin && url.pathname === '/__/firebase/init.json')
        return new Response(JSON.stringify(config));
      if (
        url.origin !== 'https://firestore.googleapis.com' ||
        !url.pathname.startsWith(`/v1/projects/${config.projectId}/databases/(default)/documents`)
      )
        throw Error('unexpected provider endpoint');
      return fetch(`http://127.0.0.1:${endpoints.firestorePort}${url.pathname}`, options);
    }) as typeof fetch,
    async load() {
      return {
        async initialize(value) {
          const port = await initialize(value);
          const app = getApp(
            JSON.stringify([value.projectId, value.appId, value.authDomain, value.apiKey]),
          );
          connectAuthEmulator(getAuth(app), `http://127.0.0.1:${endpoints.authPort}`, {
            disableWarnings: true,
          });
          const firestore = getFirestore(app);
          connectFirestoreEmulator(firestore, '127.0.0.1', endpoints.firestorePort);
          return port;
        },
      };
    },
  });
  const uid = await auth.signInAnonymous();
  const call = (): SyncCall => ({
    signal: new AbortController().signal,
    deadlineAtMs: Date.now() + 10000,
  });
  return {
    uid,
    async bind(space: string) {
      sync = await auth.sealedSync(space, {
        async admit(cost) {
          costs.push(cost);
        },
      });
    },
    async read(resource: string, id: string) {
      const d = await sync!.read({ resource, id }, call());
      return d ? { ...d, bytes: new TextDecoder().decode(d.bytes) } : undefined;
    },
    query(q: SyncQueryRequest) {
      return sync!.query(q, call());
    },
    publish(m: SyncMutation) {
      return sync!.publish(m, call());
    },
    async write(
      mode: 'create' | 'put',
      resource: string,
      id: string,
      fields: Record<string, unknown>,
    ) {
      const bytes = new TextEncoder().encode(
        JSON.stringify(
          Object.fromEntries(
            Object.entries(fields).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)),
          ),
        ),
      );
      const digest = Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', bytes)), (b) =>
        b.toString(16).padStart(2, '0'),
      ).join('');
      return sync!.publish({ mode, document: { target: { resource, id }, bytes, digest } }, call());
    },
    remove(resource: string, id: string) {
      return sync!.publish({ mode: 'delete', target: { resource, id } }, call());
    },
    async listen(interest: SyncInterest) {
      const subscription = await sync!.subscribe(interest, call());
      return { next: () => subscription.next(call()), close: () => subscription.close() };
    },
    async cancelledRead(resource: string, id: string) {
      const controller = new AbortController();
      controller.abort();
      try {
        await sync!.read(
          { resource, id },
          { signal: controller.signal, deadlineAtMs: Date.now() + 10000 },
        );
        throw Error('cancelled read returned');
      } catch (error) {
        return typeof error === 'object' && error !== null && 'code' in error
          ? error.code
          : 'unexpected';
      }
    },
    costs: () => costs,
    signOut: () => auth.signOut(),
    close: () => auth.close(),
  };
}
