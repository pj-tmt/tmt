import { createRemoteClient, type RemoteClient } from './ask-remote.js';
import type { PageTransport } from './transport.js';
import { discover, mountUrl } from './bootstrap.js';
import { register, remoteSdk } from './registration.js';
import { Live, type LiveSession, type LiveSessionOwner } from './live.js';
import { requireValue } from '@tmt/colab-client';
import { verifyRegistration } from './registration.js';
import { text } from './strings.js';

export async function mountedTransport(): Promise<{ space: string; transport: PageTransport }> {
  const mount = mountUrl(),
    sdk = await remoteSdk(),
    registration = await register(mount, sdk),
    bootstrap = await discover(mount, (space, owner) =>
      verifyRegistration(registration, space, owner),
    );
  let current: LiveSession;
  let replacement: Promise<LiveSession> | undefined;
  async function attach(registration: LiveSession['registration']): Promise<LiveSession> {
    // Consume this exact verified Session before opening any sync Connection.
    let remote: RemoteClient | null = null;
    try {
      remote = await createRemoteClient(mount, undefined, registration.remoteSession);
    } catch {
      // Old SDKs keep source usable while Ask remains unavailable.
    }
    return { registration, remote };
  }
  current = await attach(registration);
  const owner: LiveSessionOwner = {
    reconnect(previous) {
      if (previous !== current.registration) return Promise.resolve(current);
      if (replacement) return replacement;
      replacement = (async () => {
        const next = await register(mount, sdk);
        await verifyRegistration(next, bootstrap.space, bootstrap.owner);
        requireValue(next.deviceId === previous.deviceId);
        current = await attach(next);
        return current;
      })().finally(() => {
        replacement = undefined;
      });
      return replacement;
    },
  };
  return {
    space: bootstrap.space,
    transport: {
      async spaceHome() {
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
        const page = bootstrap.pages.find((page) => page.pageId === id && !page.archived);
        if (!page) throw new Error('Page unavailable');
        const live = new Live(
          mount,
          bootstrap,
          current.registration,
          page,
          signal,
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
