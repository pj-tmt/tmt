import { createRemoteClient, type RemoteClient } from './ask-remote.js';
import {
  ManagementClient,
  ManagementError,
  project,
  type ManagementPort,
  type ManagementView,
  type Pending,
} from './management.js';
import type { PageTransport } from './transport.js';
import { discover, mountUrl } from './bootstrap.js';
import { register, remoteSdk } from './registration.js';
import { Live, type LiveSession, type LiveSessionOwner } from './live.js';
import { requireValue } from '@tmt/colab-client';
import { verifyRegistration } from './registration.js';
import { clearRecovery, recoverSession } from './session-recovery.js';
import { text } from './strings.js';
import { LocalDraftStore } from './draft-store.js';
import { TitleCache } from './title-cache.js';

export async function mountedTransport(signal?: AbortSignal): Promise<{
  space: string;
  transport: PageTransport;
  close(): void;
}> {
  const lifetime = new AbortController();
  const abort = () => lifetime.abort(signal?.reason);
  signal?.addEventListener('abort', abort, { once: true });
  if (signal?.aborted) abort();
  async function owned<T>(action: () => Promise<T>): Promise<T> {
    lifetime.signal.throwIfAborted();
    const result = await action();
    lifetime.signal.throwIfAborted();
    return result;
  }
  const mount = mountUrl(),
    pairedSdk = await remoteSdk();
  lifetime.signal.throwIfAborted();
  const sdk = {
      reopenSession: () => owned(() => pairedSdk.reopenSession()),
      certifyKey: (purpose: 'sign' | 'enc', key: Uint8Array) =>
        owned(() => pairedSdk.certifyKey(purpose, key)),
      transportUrl: (session: unknown, url: string | URL) => pairedSdk.transportUrl(session, url),
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
  const titles = new TitleCache(bootstrap.space, registration.deviceId);
  const drafts = new LocalDraftStore(bootstrap.space, registration.deviceId);
  let managementClient = new ManagementClient(mount, current.registration);
  const views = new WeakMap<ManagementView, ManagementClient>();
  const requests = new WeakMap<Pending, ManagementClient>();
  const scopedSignal = (signal?: AbortSignal) =>
    signal ? AbortSignal.any([signal, lifetime.signal]) : lifetime.signal;
  function managed<T>(client: ManagementClient, action: () => Promise<T>): Promise<T> {
    return owned(async () => {
      if (client !== managementClient) throw new ManagementError('DENIED');
      const result = await action();
      lifetime.signal.throwIfAborted();
      if (client !== managementClient) throw new ManagementError('DENIED');
      return result;
    });
  }
  const management: ManagementPort = {
    read(id, signal) {
      const client = managementClient;
      return managed(client, async () => {
        const view = await client.read(id, scopedSignal(signal));
        views.set(view, client);
        return view;
      });
    },
    prepare(view, selection) {
      const client = managementClient;
      return managed(client, async () => {
        if (views.get(view) !== client) throw new ManagementError('DENIED');
        const pending = await client.prepare(view, selection);
        requests.set(pending, client);
        return pending;
      });
    },
    send(pending) {
      const client = managementClient;
      return managed(client, () => {
        if (requests.get(pending) !== client) throw new ManagementError('DENIED');
        return client.send(pending);
      });
    },
    verify(pending, ack, signal) {
      const client = managementClient;
      return managed(client, async () => {
        const view = await client.verify(pending, ack, scopedSignal(signal));
        views.set(view, client);
        return view;
      });
    },
  };
  clearRecovery(mount);
  const owner: LiveSessionOwner = {
    async rememberTitle(page, title, registration) {
      if (registration !== current.registration || lifetime.signal.aborted) return;
      try {
        await owned(() => titles.remember(page, title, lifetime.signal));
      } catch {
        // A closed tab can discard a display hint without failing Live.
      }
    },
    recover: () =>
      owned(() =>
        recoverSession({
          explicit: true,
          mount,
          storage: sessionStorage,
          reopen: () => pairedSdk.reopenSession(),
          reload: () => {
            lifetime.signal.throwIfAborted();
            location.reload();
          },
        }),
      ),
    reconnect(previous) {
      if (lifetime.signal.aborted) return Promise.reject(lifetime.signal.reason);
      if (previous !== current.registration) return Promise.resolve(current);
      if (replacement) return replacement;
      replacement = owned(async () => {
        const next = await register(mount, sdk);
        await verifyRegistration(next, bootstrap.space, bootstrap.owner);
        requireValue(next.deviceId === previous.deviceId);
        current = await attach(next);
        managementClient = new ManagementClient(mount, next);
        return current;
      }).finally(() => {
        replacement = undefined;
      });
      return replacement;
    },
  };
  return {
    space: bootstrap.space,
    close: () => {
      signal?.removeEventListener('abort', abort);
      lifetime.abort();
    },
    transport: {
      get backendName() {
        return current.registration.deviceName;
      },
      drafts,
      management,
      async spaceHome() {
        const client = managementClient;
        return managed(client, async () => {
          const { boot, log } = await client.snapshot(lifetime.signal);
          return {
            title: text.product,
            pageIds: boot.pageIds,
            pages: await Promise.all(
              boot.pages.map(async (page) => {
                const policy = project(page, log).page;
                return {
                  id: page.pageId,
                  title: (await titles.read(page.pageId)) || text.unknownPageTitle,
                  sharing: policy.sharing,
                  archived: policy.archived,
                  retentionDays: policy.retentionDays,
                  lastUpdateAtMs: policy.lastUpdateAtMs,
                  expiresAtMs: policy.expiresAtMs,
                  warnings: policy.warnings,
                };
              }),
            ),
          };
        });
      },
      async page(id, signal) {
        const bound = current;
        const boot = await owned(() =>
          discover(
            mount,
            (space, owner) => verifyRegistration(bound.registration, space, owner),
            scopedSignal(signal),
          ),
        );
        lifetime.signal.throwIfAborted();
        if (bound !== current) throw new Error('Remote session replaced');
        const page = boot.pages.find((page) => page.pageId === id && !page.archived);
        if (!page) throw new Error('Page unavailable');
        const live = new Live(
          mount,
          boot,
          bound.registration,
          page,
          scopedSignal(signal),
          bound.remote,
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
