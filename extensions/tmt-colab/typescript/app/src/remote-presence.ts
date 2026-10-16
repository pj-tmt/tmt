/** Display-only projection of the admitted Remote SDK device list, never page/session authority. */
export interface RemoteDevice {
  clientId: string;
  name: string;
  kind: 'browser' | 'addon' | 'cli';
  thisBrowser: boolean;
  liveSessionCount: number;
  lastActivityAtMs: number | null;
  revoked: boolean;
  expiresAtMs: number | null;
}
export interface RemoteDevicePage {
  devices: RemoteDevice[];
  nextCursor: string | null;
}
export interface RemotePresenceSource {
  generation: object;
  devices(input: { cursor: string | null; limit: number }): Promise<RemoteDevicePage>;
}
export type RemotePresenceSnapshot =
  | { kind: 'hidden' }
  | { kind: 'unconfirmed'; atMs: number }
  | {
      kind: 'confirmed';
      atMs: number;
      count: number;
      devices: readonly RemoteDevice[];
      settingsUrl: string;
    };

/** One finite read at a time. SDK timeouts own each request; disposal joins it without reopening. */
export class RemotePresence {
  private value: RemotePresenceSnapshot = { kind: 'hidden' };
  private listeners = new Set<() => void>();
  private pending?: Promise<void>;
  private closed = false;
  constructor(
    private readonly source: () => RemotePresenceSource | undefined,
    private readonly settingsUrl: string,
    private readonly now: () => number = Date.now,
  ) {}
  snapshot = (): RemotePresenceSnapshot => this.value;
  subscribe = (changed: () => void): (() => void) => {
    this.listeners.add(changed);
    return () => this.listeners.delete(changed);
  };
  private publish(value: RemotePresenceSnapshot) {
    this.value = value;
    for (const changed of this.listeners) changed();
  }
  refresh = (): Promise<void> => {
    if (this.closed) return Promise.resolve();
    if (this.pending) return this.pending;
    const source = this.source();
    if (!source) {
      this.publish({ kind: 'hidden' });
      return Promise.resolve();
    }
    const current = () => !this.closed && this.source()?.generation === source.generation;
    const read = async () => {
      const devices: RemoteDevice[] = [];
      const identities = new Set<string>();
      const cursors = new Set<string>();
      let cursor: string | null = null;
      try {
        for (let page = 0; page < 20; page++) {
          const result = await source.devices({ cursor, limit: 50 });
          if (!current()) return;
          if (result.devices.length > 50) throw new Error('Invalid device page');
          for (const device of result.devices) {
            if (identities.has(device.clientId)) throw new Error('Repeated device');
            identities.add(device.clientId);
            // Only omit inactive tombstones. This browser's wall clock cannot
            // overrule a positive count observed by the door's Session owner.
            const inactive = device.liveSessionCount === 0 && device.lastActivityAtMs === null;
            const expired = device.expiresAtMs !== null && device.expiresAtMs <= this.now();
            if (!(inactive && (device.revoked || expired))) devices.push(device);
          }
          if (result.nextCursor === null) {
            const count = devices.reduce((sum, device) => sum + device.liveSessionCount, 0);
            if (!Number.isSafeInteger(count) || count < 0) throw new Error('Invalid count');
            devices.sort(
              (a, b) =>
                Number(b.thisBrowser) - Number(a.thisBrowser) ||
                (b.lastActivityAtMs ?? -1) - (a.lastActivityAtMs ?? -1) ||
                a.clientId.localeCompare(b.clientId),
            );
            this.publish({
              kind: 'confirmed',
              count,
              devices,
              atMs: this.now(),
              settingsUrl: this.settingsUrl,
            });
            return;
          }
          if (cursors.has(result.nextCursor)) throw new Error('Repeated cursor');
          cursors.add(result.nextCursor);
          cursor = result.nextCursor;
        }
      } catch {
        // A refusal, timeout or incomplete traversal is not a zero or a partial total.
      }
      if (current()) this.publish({ kind: 'unconfirmed', atMs: this.now() });
    };
    this.pending = read().finally(() => {
      this.pending = undefined;
    });
    return this.pending;
  };
  invalidate(): void {
    this.publish({ kind: 'hidden' });
    // A replacement Session gets its mount read after the stale read has settled.
    void (this.pending ?? Promise.resolve()).then(() => this.refresh());
  }
  async close(): Promise<void> {
    this.closed = true;
    this.publish({ kind: 'hidden' });
    await this.pending;
    this.listeners.clear();
  }
}
