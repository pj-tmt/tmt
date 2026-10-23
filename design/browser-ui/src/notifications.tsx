import { useLayoutEffect, useRef, useState, useSyncExternalStore, type MouseEvent } from 'react';
import { X } from 'lucide-react';
import { BrowserAction } from './action.js';
import { BrowserIconAction } from './icon-action.js';
import { admitsOverlayEscape, restoreOverlayFocus } from './overlay.js';
import { NotificationQueue } from './notification-queue.js';
import type { BrowserNotificationData } from './static.js';

interface Closing {
  event: BrowserNotification;
  index: number;
}

export interface BrowserNotification extends BrowserNotificationData {
  onActivate(event: MouseEvent<HTMLButtonElement>): void;
}

export interface BrowserNotificationsProps {
  /** Changing the host page identity resets presentation and duplicate memory. */
  pageKey: string;
  events: readonly BrowserNotification[];
  suspended: boolean;
  isEligible(event: BrowserNotification): boolean;
}

export function BrowserNotifications(props: BrowserNotificationsProps) {
  return <NotificationSurface key={props.pageKey} {...props} />;
}

function NotificationSurface({ events, suspended, isEligible }: BrowserNotificationsProps) {
  const [queue] = useState(() => new NotificationQueue<BrowserNotification>());
  const surface = useRef<HTMLDivElement>(null);
  const focusOrigin = useRef<Element | null>(null);
  const host = useRef({ suspended, isEligible });
  host.current = { suspended, isEligible };
  const [announcements, setAnnouncements] = useState<readonly BrowserNotification[]>([]);
  const snapshot = useSyncExternalStore(queue.subscribe, queue.getSnapshot, queue.getSnapshot);
  // A dismissed or expired card stays mounted as closed while its exit plays, then goes.
  const [previous, setPrevious] = useState(snapshot.visible);
  const [closing, setClosing] = useState<ReadonlyMap<string, Closing>>(new Map());
  if (previous !== snapshot.visible) {
    setPrevious(snapshot.visible);
    const gone = previous.filter(
      (event) =>
        !snapshot.visible.some((visible) => visible.id === event.id) && !closing.has(event.id),
    );
    if (gone.length)
      setClosing(
        new Map([
          ...closing,
          ...gone.map((event): [string, Closing] => [
            event.id,
            { event, index: previous.indexOf(event) },
          ]),
        ]),
      );
  }
  const settling = useRef(new Set<string>());
  useLayoutEffect(() => {
    const finish = (id: string) =>
      setClosing((current) => {
        if (!current.has(id)) return current;
        const next = new Map(current);
        next.delete(id);
        return next;
      });
    for (const id of closing.keys()) {
      if (settling.current.has(id)) continue;
      settling.current.add(id);
      const running =
        surface.current
          ?.querySelector<HTMLElement>(`[data-notification="${CSS.escape(id)}"]`)
          ?.getAnimations?.() ?? [];
      const done = () => {
        settling.current.delete(id);
        finish(id);
      };
      if (running.length) void Promise.allSettled(running.map((a) => a.finished)).then(done);
      else done();
    }
  }, [closing]);
  const cards: { event: BrowserNotification; open: boolean }[] = snapshot.visible.map((event) => ({
    event,
    open: true,
  }));
  for (const { event, index } of [...closing.values()].sort((a, b) => a.index - b.index))
    cards.splice(Math.min(index, cards.length), 0, { event, open: false });
  function dismiss(id: string, target: HTMLElement) {
    const focused = target
      .closest('.tmt-ui-notification')!
      .contains(target.ownerDocument.activeElement);
    queue.remove(id);
    if (focused) restoreOverlayFocus(focusOrigin.current);
  }

  useLayoutEffect(() => {
    queue.start();
    const document = surface.current!.ownerDocument;
    const visibility = () => {
      // The queue stops synchronously; React then hides the surface before paint.
      queue.setSuspended(document.hidden || host.current.suspended, host.current.isEligible);
    };
    document.addEventListener('visibilitychange', visibility);
    return () => {
      document.removeEventListener('visibilitychange', visibility);
      queue.stop();
    };
  }, [queue]);
  useLayoutEffect(() => {
    queue.setSuspended(suspended || surface.current!.ownerDocument.hidden, isEligible);
    queue.admit(events);
  }, [queue, events, suspended, isEligible]);
  useLayoutEffect(() => {
    const node = surface.current!;
    if ((queue.getSnapshot().visible.length || closing.size) && !suspended) {
      if (!node.matches(':popover-open')) node.showPopover();
      const next = queue.takeAnnouncements();
      if (next.length) setAnnouncements(next);
      // The status holds the currently eligible payload: an active withdrawal drops its item
      // (clearing an emptied status), and surviving cards are never spoken again.
      else
        setAnnouncements((previous) => {
          const kept = previous.filter((event) => host.current.isEligible(event));
          return kept.length === previous.length ? previous : kept;
        });
    } else {
      if (node.matches(':popover-open')) node.hidePopover();
      setAnnouncements((previous) => (previous.length ? [] : previous));
      if (closing.size) setClosing(new Map());
    }
  }, [queue, snapshot, suspended, closing]);

  return (
    <>
      <div
        className="tmt-ui-notification-status"
        role="status"
        aria-live="polite"
        aria-atomic="true"
      >
        {!suspended &&
          snapshot.visible.length > 0 &&
          announcements.map((event) => (
            <span key={event.id}>{`${event.destination}: ${event.firstLine}\n`}</span>
          ))}
      </div>
      <div
        ref={surface}
        popover="manual"
        className="tmt-ui-notifications"
        onFocusCapture={(event) => {
          if (!event.currentTarget.contains(event.relatedTarget)) {
            focusOrigin.current =
              event.relatedTarget instanceof Element ? event.relatedTarget : null;
          }
        }}
      >
        {cards.map(({ event, open }) => (
          <article
            key={event.id}
            className="tmt-ui-notification"
            data-notification={event.id}
            data-state={open ? 'open' : 'closed'}
            inert={open ? undefined : true}
            aria-label={`${event.destination} reply`}
            onMouseEnter={() => queue.pause(event.id, 'hover', true)}
            onMouseLeave={() => queue.pause(event.id, 'hover', false)}
            onFocus={() => queue.pause(event.id, 'focus', true)}
            onBlur={(focusEvent) => {
              if (!focusEvent.currentTarget.contains(focusEvent.relatedTarget)) {
                queue.pause(event.id, 'focus', false);
              }
            }}
            onKeyDown={(keyEvent) => {
              if (
                keyEvent.key === 'Escape' &&
                !keyEvent.defaultPrevented &&
                admitsOverlayEscape(keyEvent)
              ) {
                keyEvent.preventDefault();
                keyEvent.stopPropagation();
                dismiss(event.id, keyEvent.currentTarget);
              }
            }}
          >
            <div className="tmt-ui-notification-heading">
              <span className="tmt-ui-notification-tag">{event.destination}</span>
              <BrowserIconAction
                type="button"
                variant="text"
                label={`Dismiss ${event.destination} reply`}
                icon={<X />}
                onActivate={(activation) => {
                  if (activation.isTrusted) dismiss(event.id, activation.currentTarget);
                }}
              />
            </div>
            <p className="tmt-ui-notification-line">{event.firstLine}</p>
            {event.destination === 'Thread' && event.quote && (
              <blockquote className="tmt-ui-notification-quote">{event.quote}</blockquote>
            )}
            <BrowserAction
              type="button"
              variant="text"
              label={`Open ${event.destination}`}
              onActivate={(activation) => {
                if (activation.isTrusted) queue.remove(event.id)?.onActivate(activation);
              }}
            />
          </article>
        ))}
        {snapshot.waiting > 0 && (
          <p className="tmt-ui-notification-waiting">{snapshot.waiting} more waiting</p>
        )}
      </div>
    </>
  );
}
