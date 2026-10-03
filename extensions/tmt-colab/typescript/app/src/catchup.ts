import { decimal, exactKeys, requireValue, strictJson, text } from '@tmt/colab-client';
import { Admission } from './admission.js';

/** Metadata-only bootstrap; stream objects are refused until live adapter adoption. */
export class Catchup {
  #started = false;
  #membershipMore = false;
  #complete = false;
  constructor(
    readonly admission: Admission,
    readonly sharing: string,
  ) {}
  async admit(raw: string): Promise<boolean> {
    const v = strictJson(text(raw), 64 * 1024, true),
      a = this.admission;
    const keys = ['version', 'type', 'space', 'page', 'epoch', 'streams', 'more'];
    requireValue(v !== null && typeof v === 'object' && !Array.isArray(v));
    const value = v as Record<string, unknown>;
    if (value.type === 'error') {
      exactKeys(value, ['version', 'type', 'space', 'page', 'epoch', 'code']);
      this.#scope(value);
      requireValue(
        typeof value.code === 'string' &&
          [
            'DENIED',
            'EXPIRED',
            'STALE_EPOCH',
            'INVALID',
            'GAP',
            'CAPACITY',
            'CONFLICT',
            'RESYNC_REQUIRED',
          ].includes(value.code),
      );
      throw new Error(`Sync bootstrap rejected: ${value.code}`);
    }
    requireValue(!this.#complete && value.type === 'catchup');
    for (const key of ['membershipHead', 'baseline', 'membership', 'chains', 'wraps'])
      if (Object.hasOwn(value, key)) keys.push(key);
    exactKeys(value, keys);
    this.#scope(value);
    requireValue(
      Array.isArray(value.streams) &&
        value.streams.length <= 256 &&
        typeof value.more === 'boolean',
    );
    if (!this.#started) {
      requireValue(
        Object.hasOwn(value, 'membershipHead') &&
          Object.hasOwn(value, 'baseline') &&
          !Object.hasOwn(value, 'membership') &&
          value.streams.length === 0 &&
          value.more,
      );
      await a.membership(value.membershipHead, true);
      this.#membershipMore = (value.membershipHead as { more: boolean }).more;
      if (value.baseline !== null) {
        // Baseline object retrieval/admission is #1157/#1252, never fall back to old source.
        throw new Error('Epoch baseline loading is not available yet');
      }
      this.#started = true;
    } else {
      requireValue(!Object.hasOwn(value, 'membershipHead') && !Object.hasOwn(value, 'baseline'));
      if (this.#membershipMore) {
        requireValue(
          Object.hasOwn(value, 'membership') && value.streams.length === 0 && value.more,
        );
        await a.membership(value.membership, false);
        this.#membershipMore = (value.membership as { more: boolean }).more;
      } else requireValue(!Object.hasOwn(value, 'membership'));
    }
    if (Object.hasOwn(value, 'chains') || Object.hasOwn(value, 'wraps'))
      requireValue(!this.#membershipMore);
    if (Object.hasOwn(value, 'chains')) await a.chains(value.chains);
    if (Object.hasOwn(value, 'wraps')) await a.wraps(value.wraps);
    if (value.streams.length) throw new Error('Live page loading is not available yet');
    if (!value.more) {
      requireValue(!this.#membershipMore && a.head !== null);
      a.validatePage(this.sharing);
      this.#complete = true;
    }
    return this.#complete;
  }
  #scope(value: Record<string, unknown>) {
    requireValue(
      value.version === 1 &&
        value.space === this.admission.space &&
        value.page === this.admission.page &&
        value.epoch === this.admission.epoch,
    );
  }
}
/** Short-lived metadata probe, not the live PageTransport adapter. */
export async function bootstrapPage(
  mount: URL,
  a: Admission,
  sharing: string,
  signal?: AbortSignal,
): Promise<void> {
  await a.restore();
  signal?.throwIfAborted();
  const url = new URL('sync', mount);
  url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
  const socket = new WebSocket(url, 'colab-sync-v1'),
    catchup = new Catchup(a, sharing);
  let tasks = Promise.resolve();
  try {
    await new Promise<void>((resolve, reject) => {
      let stopped = false,
        queued = 0;
      const timer = setTimeout(() => finish(new Error('Sync bootstrap timed out')), 10_000);
      const abort = () => finish(new Error('Sync bootstrap cancelled'));
      signal?.addEventListener('abort', abort, { once: true });
      function finish(error?: Error) {
        if (stopped) return;
        stopped = true;
        clearTimeout(timer);
        signal?.removeEventListener('abort', abort);
        socket.onopen = socket.onmessage = socket.onerror = socket.onclose = null;
        socket.close();
        if (error) reject(error);
        else resolve();
      }
      socket.onopen = () => {
        if (socket.protocol !== 'colab-sync-v1') return finish(new Error('Invalid sync protocol'));
        const revision = a.head?.revision.toString() ?? '0';
        decimal(revision, true);
        socket.send(
          JSON.stringify({
            version: 1,
            type: 'hello',
            space: a.space,
            page: a.page,
            epoch: a.epoch,
            device: a.registration.deviceId,
            membershipRevision: revision,
            cursors: [],
          }),
        );
      };
      socket.onmessage = (event: MessageEvent<unknown>) => {
        if (stopped) return;
        if (typeof event.data !== 'string' || text(event.data).length > 64 * 1024 || ++queued > 8)
          return finish(new Error('Sync bootstrap capacity'));
        const raw = event.data;
        tasks = tasks
          .then(async () => {
            if (stopped) return;
            const complete = await catchup.admit(raw);
            queued--;
            if (complete) finish();
          })
          .catch(() => finish(new Error('Sync bootstrap unavailable or invalid')));
      };
      socket.onerror = socket.onclose = () => finish(new Error('Sync bootstrap disconnected'));
    });
  } finally {
    await tasks;
  }
}
