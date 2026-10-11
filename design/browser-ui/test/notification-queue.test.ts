import { describe, expect, it } from 'vite-plus/test';
import {
  NotificationQueue,
  type NotificationClock,
  type BrowserNotificationData,
} from '../src/notification-queue.js';

function fixture() {
  let time = 0;
  let serial = 0;
  const timers = new Map<number, { at: number; callback: () => void }>();
  const clock: NotificationClock = {
    now: () => time,
    setTimeout(callback, delay) {
      const id = ++serial;
      timers.set(id, { at: time + delay, callback });
      return id;
    },
    clearTimeout: (id) => {
      timers.delete(id as number);
    },
  };
  const queue = new NotificationQueue<BrowserNotificationData>(clock);
  queue.start();
  queue.setSuspended(false, () => true);
  return {
    queue,
    timers,
    advance(ms: number) {
      const end = time + ms;
      for (;;) {
        const next = [...timers].sort((a, b) => a[1].at - b[1].at)[0];
        if (!next || next[1].at > end) break;
        time = next[1].at;
        timers.delete(next[0]);
        next[1].callback();
      }
      time = end;
    },
  };
}
const events = (count: number, offset = 0): BrowserNotificationData[] =>
  Array.from({ length: count }, (_, index) => ({
    id: String(index + offset + 1),
    destination: 'Thread',
    firstLine: `Reply ${index + offset + 1}`,
  }));
const ids = (queue: NotificationQueue<BrowserNotificationData>) =>
  queue.getSnapshot().visible.map((value) => value.id);

describe('notification presentation queue', () => {
  it('keeps FIFO slots and gives waiting entries a full budget on promotion', () => {
    const { queue, advance } = fixture();
    queue.admit(events(5));
    expect(ids(queue)).toEqual(['1', '2', '3']);
    expect(queue.getSnapshot().waiting).toBe(2);
    expect(queue.takeAnnouncements().map((value) => value.id)).toEqual(['1', '2', '3']);
    advance(5999);
    expect(ids(queue)).toEqual(['1', '2', '3']);
    advance(1);
    expect(ids(queue)).toEqual(['4', '5']);
    expect(queue.getSnapshot().waiting).toBe(0);
    expect(queue.takeAnnouncements().map((value) => value.id)).toEqual(['4', '5']);
    advance(5999);
    expect(ids(queue)).toEqual(['4', '5']);
    advance(1);
    expect(ids(queue)).toEqual([]);
  });
  it('deduplicates even after dismissal and expiry, but a new page has new memory', () => {
    const { queue, advance } = fixture();
    queue.admit(events(3));
    queue.remove('1');
    advance(6000);
    queue.admit(events(3));
    expect(ids(queue)).toEqual([]);
    const nextPage = fixture().queue;
    nextPage.admit(events(1));
    expect(ids(nextPage)).toEqual(['1']);
  });
  it('preserves independent budgets and overlapping hover/focus pauses', () => {
    const { queue, advance } = fixture();
    queue.admit(events(2));
    advance(2000);
    queue.pause('1', 'hover', true);
    queue.pause('1', 'focus', true);
    advance(4000);
    expect(ids(queue)).toEqual(['1']);
    queue.pause('1', 'hover', false);
    advance(10000);
    expect(ids(queue)).toEqual(['1']);
    queue.pause('1', 'focus', false);
    advance(3999);
    expect(ids(queue)).toEqual(['1']);
    advance(1);
    expect(ids(queue)).toEqual([]);
  });
  it('hides and freezes existing entries, retaining their elapsed budget and announcement memory', () => {
    const { queue, advance } = fixture();
    queue.admit(events(1));
    queue.takeAnnouncements();
    advance(2500);
    queue.setSuspended(true, () => true);
    queue.admit(events(1, 1));
    expect(ids(queue)).toEqual([]);
    expect(queue.takeAnnouncements()).toEqual([]);
    advance(30000);
    queue.setSuspended(false, () => true);
    expect(ids(queue)).toEqual(['1', '2']);
    expect(queue.takeAnnouncements().map((value) => value.id)).toEqual(['2']);
    advance(3500);
    expect(ids(queue)).toEqual(['2']);
    advance(2500);
    expect(ids(queue)).toEqual([]);
  });
  it('rechecks every queued entry before resume: five admitted, two withdrawn, three announced', () => {
    const { queue, advance, timers } = fixture();
    queue.setSuspended(true, () => true);
    queue.admit(events(5));
    advance(10000);
    expect(timers.size).toBe(0);
    const checked: string[] = [];
    queue.setSuspended(false, (event) => {
      checked.push(event.id);
      return event.id !== '1' && event.id !== '4';
    });
    expect(checked).toEqual(['1', '2', '3', '4', '5']);
    expect(ids(queue)).toEqual(['2', '3', '5']);
    expect(queue.getSnapshot().waiting).toBe(0);
    expect(queue.takeAnnouncements().map((value) => value.id)).toEqual(['2', '3', '5']);
    expect(queue.takeAnnouncements()).toEqual([]);
  });
  it('filters retained and pending entries together and does not reannounce survivors', () => {
    const { queue } = fixture();
    queue.admit(events(5));
    queue.takeAnnouncements();
    queue.setSuspended(true, () => true);
    queue.setSuspended(false, (event) => event.id !== '2' && event.id !== '5');
    expect(ids(queue)).toEqual(['1', '3', '4']);
    expect(queue.takeAnnouncements().map((value) => value.id)).toEqual(['4']);
  });
  it('removes before activation and promotes exactly once, without renewing other budgets', () => {
    const { queue, advance } = fixture();
    queue.admit(events(4));
    advance(2000);
    expect(queue.remove('1')?.id).toBe('1');
    expect(queue.remove('1')).toBeUndefined();
    expect(ids(queue)).toEqual(['2', '3', '4']);
    advance(4000);
    expect(ids(queue)).toEqual(['4']);
    advance(2000);
    expect(ids(queue)).toEqual([]);
  });
  it('cancels teardown timers and supports an effect replay without losing elapsed time', () => {
    const { queue, advance, timers } = fixture();
    queue.admit(events(1));
    advance(2000);
    queue.stop();
    expect(timers.size).toBe(0);
    advance(10000);
    queue.start();
    advance(3999);
    expect(ids(queue)).toEqual(['1']);
    advance(1);
    expect(ids(queue)).toEqual([]);
  });
  it('clears obsolete pointer/focus pauses when the surface is suspended', () => {
    const { queue, advance } = fixture();
    queue.admit(events(1));
    queue.pause('1', 'focus', true);
    queue.setSuspended(true, () => true);
    queue.setSuspended(false, () => true);
    advance(6000);
    expect(ids(queue)).toEqual([]);
  });
});
