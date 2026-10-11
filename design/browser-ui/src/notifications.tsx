import { useLayoutEffect, useRef, useState, useSyncExternalStore, type MouseEvent } from 'react';
import { X } from 'lucide-react';
import { BrowserAction } from './action.js';
import { BrowserIconAction } from './icon-action.js';
import { admitsOverlayEscape } from './overlay.js';
import { NotificationQueue, type BrowserNotificationData } from './notification-queue.js';

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
  const host = useRef({ suspended, isEligible });
  host.current = { suspended, isEligible };
  const [announcements, setAnnouncements] = useState<readonly BrowserNotification[]>([]);
  const snapshot = useSyncExternalStore(queue.subscribe, queue.getSnapshot, queue.getSnapshot);

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
    if (queue.getSnapshot().visible.length && !suspended) {
      if (!node.matches(':popover-open')) node.showPopover();
      const next = queue.takeAnnouncements();
      if (next.length) setAnnouncements(next);
    } else {
      if (node.matches(':popover-open')) node.hidePopover();
      setAnnouncements((previous) => (previous.length ? [] : previous));
    }
  }, [queue, snapshot, suspended]);

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
      <div ref={surface} popover="manual" className="tmt-ui-notifications">
        {snapshot.visible.map((event) => (
          <article
            key={event.id}
            className="tmt-ui-notification"
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
                queue.remove(event.id);
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
                  if (activation.isTrusted) queue.remove(event.id);
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
