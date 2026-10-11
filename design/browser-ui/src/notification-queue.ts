export interface BrowserNotificationData {
  id: string;
  destination: 'Chat' | 'Thread' | 'Answer' | 'Proposal';
  firstLine: string;
  quote?: string;
}

export interface NotificationClock {
  now(): number;
  setTimeout(callback: () => void, delay: number): unknown;
  clearTimeout(timer: unknown): void;
}

export const notificationClock: NotificationClock = {
  now: () => performance.now(),
  setTimeout: (callback, delay) => globalThis.setTimeout(callback, delay),
  clearTimeout: (timer) => globalThis.clearTimeout(timer as ReturnType<typeof setTimeout>),
};

interface Entry<T> {
  event: T;
  remaining: number;
  deadline?: number;
  pauses: Set<'hover' | 'focus'>;
  announced: boolean;
}

/** Presentation memory only. A host page key gives each controller its lifetime. */
export class NotificationQueue<T extends BrowserNotificationData> {
  private entries: Entry<T>[] = [];
  private seen = new Set<string>();
  private listeners = new Set<() => void>();
  private timer: unknown;
  private running = false;
  private suspended = true;
  private eligible: (event: T) => boolean = () => true;
  private snapshot: { visible: readonly T[]; waiting: number } = { visible: [], waiting: 0 };

  constructor(private clock: NotificationClock = notificationClock) {}

  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };
  getSnapshot = () => this.snapshot;

  start() {
    this.running = true;
    this.refresh();
  }

  stop() {
    this.freeze();
    this.running = false;
  }

  setSuspended(suspended: boolean, eligible: (event: T) => boolean) {
    this.freeze();
    this.eligible = eligible;
    // Filter the entire queue before a resumed surface can display or announce it.
    if (!suspended) this.entries = this.entries.filter((entry) => eligible(entry.event));
    else for (const entry of this.entries) entry.pauses.clear();
    this.suspended = suspended;
    this.refresh();
  }

  admit(events: readonly T[]) {
    this.freeze();
    for (const event of events) {
      if (this.seen.has(event.id)) continue;
      this.seen.add(event.id);
      if (!this.suspended && !this.eligible(event)) continue;
      this.entries.push({ event, remaining: 6000, pauses: new Set(), announced: false });
    }
    this.refresh();
  }

  remove(id: string): T | undefined {
    if (this.suspended) return;
    this.freeze();
    const entry = this.entries.find((value) => value.event.id === id);
    this.entries = this.entries.filter((value) => value.event.id !== id);
    this.refresh();
    return entry?.event;
  }

  pause(id: string, reason: 'hover' | 'focus', paused: boolean) {
    this.freeze();
    const entry = this.entries.find((value) => value.event.id === id);
    if (paused) entry?.pauses.add(reason);
    else entry?.pauses.delete(reason);
    this.refresh();
  }

  takeAnnouncements(): readonly T[] {
    if (this.suspended || !this.running) return [];
    return this.entries.slice(0, 3).flatMap((entry) => {
      if (entry.announced) return [];
      entry.announced = true;
      return [entry.event];
    });
  }

  private freeze() {
    if (this.timer !== undefined) this.clock.clearTimeout(this.timer);
    this.timer = undefined;
    const now = this.clock.now();
    for (const entry of this.entries) {
      if (entry.deadline !== undefined) {
        entry.remaining = Math.max(0, entry.deadline - now);
        entry.deadline = undefined;
      }
    }
    this.entries = this.entries.filter((entry) => entry.remaining > 0);
  }

  private refresh() {
    const visible = this.suspended ? [] : this.entries.slice(0, 3);
    if (this.running && !this.suspended) {
      const now = this.clock.now();
      for (const entry of visible) {
        if (!entry.pauses.size) entry.deadline = now + entry.remaining;
      }
      const delays = visible.flatMap((entry) =>
        entry.deadline === undefined ? [] : [entry.deadline - now],
      );
      if (delays.length) {
        this.timer = this.clock.setTimeout(
          () => {
            this.freeze();
            this.refresh();
          },
          Math.min(...delays),
        );
      }
    }
    this.snapshot = {
      visible: visible.map((entry) => entry.event),
      waiting: this.suspended ? 0 : Math.max(0, this.entries.length - 3),
    };
    for (const listener of this.listeners) listener();
  }
}
