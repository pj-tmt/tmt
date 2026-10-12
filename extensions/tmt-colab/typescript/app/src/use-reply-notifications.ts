import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { BrowserNotification } from '@tmt/browser-ui/react';
import type { ConversationReading, ReplyReveal } from './reply-notification-context.js';
import { threadNotificationDestination } from './reply-notification-context.js';
import {
  projectReplyNotifications,
  type ReplyNotificationObserver,
  type ReplyNotification,
  type ReplySources,
  type ReplyClassification,
} from './reply-notifications.js';

const destinationKey = (event: ReplyNotification) =>
  event.destination === 'Chat'
    ? 'Chat'
    : threadNotificationDestination(event.thread!.writer, event.thread!.id);

export function useReplyNotifications({
  pageKey,
  observer,
  sources: { asks, threads, askUnavailable },
  deviceId,
  classification: { questionThreads },
  blocked,
  activate,
}: {
  pageKey: string;
  observer: ReplyNotificationObserver;
  sources: ReplySources;
  deviceId: string | undefined;
  classification: ReplyClassification;
  blocked: boolean;
  activate(event: ReplyNotification): boolean;
}) {
  const create = () => ({
    pageKey,
    observer,
    mounted: false,
    events: [] as readonly BrowserNotification[],
  });
  const [scope, setScope] = useState(create);
  if (scope.pageKey !== pageKey || scope.observer !== observer) setScope(create());
  const [modalCount, setModalCount] = useState(0);
  const modals = useRef(new Set<string>());
  const readers = useRef(new Map<string, { destination: string; state: ConversationReading }>());
  const [, setReadingRevision] = useState(0);
  const [reveal, setReveal] = useState<ReplyReveal>();
  const projection = useMemo(
    () =>
      projectReplyNotifications({ asks, threads, askUnavailable }, deviceId, { questionThreads }),
    [asks, threads, askUnavailable, deviceId, questionThreads],
  );
  const latest = useRef({ projection, blocked, activate });
  latest.current = { projection, blocked, activate };
  const alreadyReading = useCallback(
    (event: ReplyNotification) =>
      [...readers.current.values()].some(
        (reader) =>
          reader.destination === destinationKey(event) &&
          reader.state.visible &&
          reader.state.atEnd,
      ),
    [],
  );
  const modal = useCallback((id: string, open: boolean) => {
    if (open) modals.current.add(id);
    else modals.current.delete(id);
    setModalCount(modals.current.size);
  }, []);
  const reading = useCallback(
    (id: string, destination: string, state: ConversationReading | null) => {
      const previous = readers.current.get(id);
      if (
        state &&
        previous?.destination === destination &&
        previous.state.visible === state.visible &&
        previous.state.atEnd === state.atEnd
      )
        return;
      if (!state && !previous) return;
      if (state) readers.current.set(id, { destination, state });
      else readers.current.delete(id);
      setReadingRevision((value) => value + 1);
    },
    [],
  );
  const revealed = useCallback(
    (id: string) => setReveal((current) => (current?.id === id ? undefined : current)),
    [],
  );
  useEffect(() => {
    // The router may render the preceding page view until its loader-reset effect runs.
    if (!scope.mounted) {
      scope.mounted = true;
      return;
    }
    const arrivals = scope.observer
      .observe({ asks, threads, askUnavailable }, projection)
      .filter((event) => !blocked && !alreadyReading(event));
    if (!arrivals.length) return;
    const events = arrivals.map((event): BrowserNotification => ({
      ...event,
      onActivate: () => {
        const current = latest.current.projection.candidates.get(event.id);
        if (!current || latest.current.blocked || current.revision !== event.revision) return;
        if (latest.current.activate(current))
          setReveal({
            id: event.id,
            destination: destinationKey(current),
            recordKey: current.recordKey,
          });
      },
    }));
    setScope((current) => (current.observer === scope.observer ? { ...current, events } : current));
  }, [scope, asks, threads, askUnavailable, projection, blocked, alreadyReading]);
  const isEligible = useCallback(
    (event: BrowserNotification) => {
      const current = latest.current.projection.candidates.get(event.id);
      const revision = 'revision' in event ? event.revision : undefined;
      return (
        !!current &&
        !latest.current.blocked &&
        current.revision === revision &&
        !alreadyReading(current)
      );
    },
    [alreadyReading],
  );
  return {
    // Reconcile presentation against the current projection/reading state even
    // when no new reply arrived. The leaf rechecks retained queue entries too.
    events: scope.events.filter(isEligible),
    modalCount,
    isEligible,
    context: { modal, reading, reveal, revealed },
  };
}
