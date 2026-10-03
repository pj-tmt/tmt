import { createRemoteClient, type RemoteClient } from './ask-remote.js';
import type { PageTransport } from './transport.js';
import { discover, mountUrl } from './bootstrap.js';
import { register, remoteSdk } from './registration.js';
import { Live, type LiveSession, type LiveSessionOwner } from './live.js';
import { requireValue } from '@tmt/colab-client';
import { verifyRegistration } from './registration.js';
import { text } from './strings.js';
import { InactiveTabError, type TabOwnership } from './active-tab.js';

export async function mountedTransport(
  ownership: TabOwnership,
): Promise<{ space: string; transport: PageTransport; close(): void }> {
  const lifetime = new AbortController();
  function owned<T>(action: () => Promise<T>): Promise<T> {
    if (lifetime.signal.aborted) return Promise.reject(new InactiveTabError());
    return ownership.run(action);
  }
  const mount = mountUrl(),
    pairedSdk = await remoteSdk(),
    sdk = {
      reopenSession: () => owned(() => pairedSdk.reopenSession()),
      certifyKey: (purpose: 'sign' | 'enc', key: Uint8Array) =>
        owned(() => pairedSdk.certifyKey(purpose, key)),
    },
    registration = await owned(() => register(mount, sdk)),
    bootstrap = await discover(mount, (space, owner) =>
      verifyRegistration(registration, space, owner),
    );
  let current: LiveSession;
  let replacement: Promise<LiveSession> | undefined;
  async function attach(registration: LiveSession['registration']): Promise<LiveSession> {
    // Consume this exact verified Session before opening any sync Connection.
    let remote: RemoteClient | null = null;
    try {
      const port = await createRemoteClient(mount, undefined, registration.remoteSession);
      remote = {
        context: () => owned(() => port.context()),
        listAgents: () => owned(() => port.listAgents()),
        send: (input) => owned(() => port.send(input)),
        operation: (id) => owned(() => port.operation(id)),
        result: (id) => owned(() => port.result(id)),
      };
    } catch {
      // Old SDKs keep source usable while Ask remains unavailable.
    }
    return { registration, remote };
  }
  current = await attach(registration);
  const owner: LiveSessionOwner = {
    reconnect(previous) {
      if (lifetime.signal.aborted || !ownership.active)
        return Promise.reject(new InactiveTabError());
      if (previous !== current.registration) return Promise.resolve(current);
      if (replacement) return replacement;
      replacement = owned(async () => {
        const next = await register(mount, sdk);
        await verifyRegistration(next, bootstrap.space, bootstrap.owner);
        requireValue(next.deviceId === previous.deviceId);
        current = await attach(next);
        return current;
      }).finally(() => {
        replacement = undefined;
      });
      return replacement;
    },
  };
  return {
    space: bootstrap.space,
    close: () => lifetime.abort(),
    transport: {
      async spaceHome() {
        if (lifetime.signal.aborted || !ownership.active) throw new InactiveTabError();
        return {
          title: text.product,
          pages: bootstrap.pages
            .filter((page) => !page.archived)
            .map((page) => ({
              id: page.pageId,
              title: page.pageId,
              sharing: page.sharing,
            })),
        };
      },
      async page(id, signal) {
        if (lifetime.signal.aborted || !ownership.active) throw new InactiveTabError();
        const page = bootstrap.pages.find((page) => page.pageId === id && !page.archived);
        if (!page) throw new Error('Page unavailable');
        const live = new Live(
          mount,
          bootstrap,
          current.registration,
          page,
          signal ? AbortSignal.any([signal, lifetime.signal]) : lifetime.signal,
          current.remote,
          owner,
        );
        try {
          return await live.snapshot();
        } catch (error) {
          live.close();
          throw error;
        }
      },
    },
  };
}
