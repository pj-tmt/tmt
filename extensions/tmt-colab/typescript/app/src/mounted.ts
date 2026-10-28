import { readReadingDetails } from './reading-details.js';
import { RemotePresence } from './remote-presence.js';
import { createRemoteClient, type RemoteClient } from './ask-remote.js';
import {
  ManagementClient,
  ManagementError,
  projectPolicies,
  type ManagementPort,
  type ManagementView,
  type Pending,
} from './management.js';
import type { PageTransport, SpaceHome } from './transport.js';
import { discover, mountUrl } from './bootstrap.js';
import { AdmissionRefusedError, register, remoteSdk } from './registration.js';
import { classify } from './connection-state.js';
import { Live, type LiveSession, type LiveSessionOwner } from './live.js';
import { requireValue } from '@tmt/colab-client';
import { verifyRegistration } from './registration.js';
import { clearRecovery, recoverSession } from './session-recovery.js';
import { text } from './strings.js';
import { LocalDraftStore } from './draft-store.js';
import { TitleCache } from './title-cache.js';
import { Admission } from './admission.js';
import { Connection } from './connection.js';

/**
 * The paired mount's transport. Colab's own routes read the mount from the document's location;
 * a host whose document belongs to another module (the global search) names the mount, and
 * discovery then never reads or rewrites the location.
 */
export async function mountedTransport(
  signal?: AbortSignal,
  options: { mount?: URL } = {},
): Promise<{
  space: string;
  pageIds: { pageId: string; deleted: boolean }[];
  transport: PageTransport;
  close(): void;
}> {
  const locate = options.mount === undefined;
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
  const mount = options.mount ?? mountUrl(),
    pairedSdk = await remoteSdk();
  lifetime.signal.throwIfAborted();
  const sdk = {
      reopenSession: (previous?: unknown, options?: { retry?: 'bounded'; signal?: AbortSignal }) =>
        owned(() => pairedSdk.reopenSession(previous, options)),
      certifyKey: (purpose: 'sign' | 'enc', key: Uint8Array) =>
        owned(() => pairedSdk.certifyKey(purpose, key)),
      transportUrl: (session: unknown, url: string | URL) => pairedSdk.transportUrl(session, url),
    },
    registration = await owned(() => register(mount, sdk)),
    bootstrap = await discover(
      mount,
      (space, owner) => verifyRegistration(registration, space, owner),
      undefined,
      { locate },
    );
  let current: LiveSession;
  let replacement: Promise<LiveSession> | undefined;
  let detailReads: Promise<unknown> = Promise.resolve();
  let titleGeneration = new AbortController();
  let titleReads: Promise<void> = Promise.resolve();
  let titleCleanupConfirmed = true;
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
        sendNow: (id) => owned(() => port.sendNow(id)),
      };
    } catch {
      // Old SDKs keep source usable while Ask remains unavailable.
    }
    return { registration, remote };
  }
  current = await attach(registration);
  const remotePresence = new RemotePresence(() => {
    const session = current.registration.remoteSession;
    const readManagement = pairedSdk.management;
    if (lifetime.signal.aborted || !session || !readManagement) return undefined;
    return {
      generation: current.registration,
      devices: (input) => readManagement(session, { timeoutMs: 20000 }).devices(input),
    };
  }, new URL('/settings', mount).href);
  const titles = new TitleCache(bootstrap.space, registration.deviceId);
  const drafts = new LocalDraftStore(bootstrap.space, registration.deviceId);
  let managementClient = new ManagementClient(mount, current.registration, locate);
  const views = new WeakMap<ManagementView, ManagementClient>();
  const requests = new WeakMap<Pending, ManagementClient>();
  const scopedSignal = (signal?: AbortSignal) =>
    signal ? AbortSignal.any([signal, lifetime.signal]) : lifetime.signal;
  /** The door refused a read because this device's Remote session ended and can be reopened. */
  function endedSession(error: unknown): boolean {
    return (
      error instanceof AdmissionRefusedError &&
      (error.status === 403 || error.status === 503) &&
      classify(error).recovery === 'reopen'
    );
  }
  function managed<T>(client: ManagementClient, action: () => Promise<T>): Promise<T> {
    return owned(async () => {
      if (client !== managementClient) throw new ManagementError('DENIED');
      const result = await action();
      lifetime.signal.throwIfAborted();
      if (client !== managementClient) throw new ManagementError('DENIED');
      return result;
    });
  }
  /** The Home read in flight, if any: readers that arrive while it runs share it. */
  let homeLoad: HomeLoad | undefined;
  interface HomeLoad {
    readonly promise: Promise<SpaceHome>;
    readonly cancel: AbortController;
    readers: number;
    settled: boolean;
  }
  function startHomeLoad(): HomeLoad {
    const cancel = new AbortController();
    const load: HomeLoad = {
      cancel,
      readers: 0,
      settled: false,
      promise: loadHome(scopedSignal(cancel.signal)).finally(() => {
        load.settled = true;
        if (homeLoad === load) homeLoad = undefined;
      }),
    };
    // A load every reader has left has nobody to report to.
    load.promise.catch(() => {});
    return load;
  }
  async function loadHome(readSignal: AbortSignal): Promise<SpaceHome> {
    const read = (client: ManagementClient) =>
      managed(client, async () => {
        const { boot, log } = await client.snapshot(readSignal);
        // One pass over the owner chain for every page, not one per page.
        const policies = projectPolicies(boot.pages, log);
        return {
          title: text.product,
          projects: boot.projects,
          projectPins: boot.projectPins,
          pageIds: boot.pageIds,
          pages: await Promise.all(
            boot.pages.map(async (page) => {
              const policy = policies.get(page.pageId)!;
              // The owner's own title is current; the browser's hint only covers a page the owner
              // has none for (not folded yet, or the reply had no room for it).
              const known = page.title ?? (await titles.read(page.pageId));
              return {
                id: page.pageId,
                title: known || text.unknownPageTitle,
                titleMissing: known === undefined,
                sharing: policy.sharing,
                archived: policy.archived,
                retentionDays: policy.retentionDays,
                lastUpdateAtMs: policy.lastUpdateAtMs,
                expiresAtMs: policy.expiresAtMs,
                warnings: policy.warnings,
                projects: page.projects,
                ...(page.pinned ? { pinned: true } : {}),
                summary: page.summary,
                state: page.state,
                stateError: page.stateError,
              };
            }),
          ),
        };
      });
    const client = managementClient;
    try {
      return await read(client);
    } catch (error) {
      if (!endedSession(error)) throw error;
      // The list read refreshes the same device once, like a page read, so Back from a page
      // after the session ended lands on the list.
      await owner.reconnect(client.registration, readSignal);
      return read(managementClient);
    }
  }
  const management: ManagementPort = {
    storage: {
      read(id, signal) {
        const client = managementClient;
        return managed(client, () => client.storage.read(id, scopedSignal(signal)));
      },
      reclaim(id, signal) {
        const client = managementClient;
        return managed(client, () => client.storage.reclaim(id, scopedSignal(signal)));
      },
    },
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
    setProjects(changes) {
      const client = managementClient;
      return managed(client, () => client.setProjects(changes));
    },
    manageProject(change) {
      const client = managementClient;
      return managed(client, () => client.manageProject(change));
    },
    setPinned(pageId, pinned) {
      const client = managementClient;
      return managed(client, () => client.setPinned(pageId, pinned));
    },
  };
  clearRecovery(mount);
  const owner = {
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
    reconnect(previous, readmitSignal = lifetime.signal) {
      if (readmitSignal.aborted) return Promise.reject(readmitSignal.reason);
      if (lifetime.signal.aborted) return Promise.reject(lifetime.signal.reason);
      if (previous !== current.registration) return Promise.resolve(current);
      if (replacement) return replacement;
      replacement = owned(async () => {
        const next = await register(mount, sdk, {
          previous: previous.remoteSession,
          signal: scopedSignal(readmitSignal),
        });
        readmitSignal.throwIfAborted();
        await verifyRegistration(next, bootstrap.space, bootstrap.owner);
        readmitSignal.throwIfAborted();
        requireValue(next.deviceId === previous.deviceId);
        const admitted = await attach(next);
        readmitSignal.throwIfAborted();
        titleGeneration.abort();
        titleGeneration = new AbortController();
        current = admitted;
        remotePresence.invalidate();
        managementClient = new ManagementClient(mount, next, locate);
        return current;
      }).finally(() => {
        replacement = undefined;
      });
      return replacement;
    },
  } satisfies LiveSessionOwner;
  return {
    space: bootstrap.space,
    pageIds: bootstrap.pageIds,
    close: () => {
      signal?.removeEventListener('abort', abort);
      lifetime.abort();
      void remotePresence.close();
    },
    transport: {
      get backendName() {
        return current.registration.deviceName;
      },
      drafts,
      management,
      remotePresence,
      spaceHome(signal) {
        if (signal?.aborted) return Promise.reject(signal.reason);
        const load = (homeLoad ??= startHomeLoad());
        load.readers++;
        // Each reader keeps its own signal. Leaving ends only its own wait; the load is cancelled
        // when the last reader has left before it finished. Each also gets its own copy of the
        // result, so one reader changing it cannot reach another.
        return new Promise<SpaceHome>((resolve, reject) => {
          const leave = () => {
            signal?.removeEventListener('abort', leaveAborted);
            if (--load.readers > 0 || load.settled) return;
            if (homeLoad === load) homeLoad = undefined;
            load.cancel.abort(signal?.reason);
          };
          const leaveAborted = () => {
            leave();
            reject(signal!.reason);
          };
          signal?.addEventListener('abort', leaveAborted, { once: true });
          load.promise.then(
            (home) => {
              load.readers--;
              signal?.removeEventListener('abort', leaveAborted);
              resolve(structuredClone(home));
            },
            (error: unknown) => {
              load.readers--;
              signal?.removeEventListener('abort', leaveAborted);
              reject(error);
            },
          );
        });
      },
      readingDetails(id, signal) {
        // A new shell opening joins the previous aborted request before starting its read.
        const request = detailReads.then(() =>
          owned(async () => {
            const readSignal = AbortSignal.any([signal, lifetime.signal]);
            readSignal.throwIfAborted();
            const bound = replacement ? await replacement : current;
            readSignal.throwIfAborted();
            const details = await readReadingDetails(mount, id, readSignal);
            readSignal.throwIfAborted();
            requireValue(bound === current);
            return details;
          }),
        );
        detailReads = request.catch(() => undefined);
        return request;
      },
      readTitle(id, budget, signal) {
        // A new Home generation can start while the old read is joining. One owner serializes
        // both generations, so cancellation never overlaps page folds or their key cleanup.
        const request = titleReads.then(() =>
          owned(async () => {
            const bound = replacement ? await replacement : current;
            const readSignal = AbortSignal.any([signal, lifetime.signal, titleGeneration.signal]);
            readSignal.throwIfAborted();
            const cached = await titles.read(id);
            if (cached !== undefined) return cached;
            requireValue(titleCleanupConfirmed && budget.remainingBytes > 0);
            const boot = await discover(
              mount,
              (space, owner) => verifyRegistration(bound.registration, space, owner),
              readSignal,
              { locate },
            );
            readSignal.throwIfAborted();
            requireValue(bound === current);
            const page = boot.pages.find((page) => page.pageId === id);
            if (!page) throw new Error('Page unavailable');
            const admission = new Admission(
              boot.space,
              page.pageId,
              page.epoch,
              boot.owner,
              bound.registration,
            );
            let connection: Connection | undefined;
            try {
              await admission.restore();
              readSignal.throwIfAborted();
              requireValue(bound === current);
              connection = new Connection(
                admission,
                mount,
                page.sharing,
                () => {},
                () => {},
                ['colab-sync-v1'],
                true,
                budget,
              );
              const active = connection;
              const abort = () => active.close();
              readSignal.addEventListener('abort', abort, { once: true });
              if (readSignal.aborted) abort();
              const result = await active.ready.then(
                (value) => ({ ok: true as const, title: value.title }),
                (error: unknown) => ({ ok: false as const, error }),
              );
              readSignal.removeEventListener('abort', abort);
              try {
                await active.closeAndJoin();
              } catch (error) {
                // No successor may start while this socket could still be closing.
                titleCleanupConfirmed = false;
                budget.remainingBytes = 0;
                throw error;
              }
              if (!result.ok) throw result.error;
              readSignal.throwIfAborted();
              requireValue(bound === current);
              return await titles.fillMissing(id, result.title, readSignal);
            } finally {
              if (!connection) admission.closeKeys();
            }
          }),
        );
        titleReads = request.then(
          () => {},
          () => {},
        );
        return request;
      },
      async page(id, signal) {
        const pageSignal = scopedSignal(signal);
        let bound = current;
        const read = () =>
          owned(() =>
            discover(
              mount,
              (space, owner) => verifyRegistration(bound.registration, space, owner),
              pageSignal,
              { locate },
            ),
          );
        let boot;
        try {
          boot = await read();
        } catch (error) {
          if (!endedSession(error)) throw error;
          // One same-device refresh per page attempt; PageReopener owns all retry timing.
          await owner.reconnect(bound.registration, pageSignal);
          pageSignal.throwIfAborted();
          bound = current;
          boot = await read();
        }
        pageSignal.throwIfAborted();
        if (bound !== current) throw new Error('Remote session replaced');
        const page = boot.pages.find((page) => page.pageId === id);
        if (!page) throw new Error('Page unavailable');
        const live = new Live(
          mount,
          boot,
          bound.registration,
          page,
          pageSignal,
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
