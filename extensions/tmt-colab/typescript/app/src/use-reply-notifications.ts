import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { BrowserNotification } from '@tmt/browser-ui/react';
import type { ConversationReading, ReplyReveal } from './reply-notification-context.js';
import {
  answerNotificationDestination,
  proposalNotificationDestination,
  threadNotificationDestination,
} from './reply-notification-context.js';
import {
  projectReplyNotifications,
  type ReplyNotificationObserver,
  type ReplyNotification,
  type ReplySources,
  type ReplyClassification,
} from './reply-notifications.js';

const destinationKey = (event: ReplyNotification) => {
  if (event.destination === 'Chat') return 'Chat';
  if (event.destination === 'Answer') return answerNotificationDestination(event.questionId!);
  return (
    event.destination === 'Proposal'
      ? proposalNotificationDestination
      : threadNotificationDestination
  )(event.thread!.writer, event.thread!.id);
};
/** A click never becomes an unbounded future navigation after its source changes. */
export const REPLY_REVEAL_WAIT_MS = 5000;

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
  const [reveal, setReveal] = useState<{
    pageKey: string;
    event: ReplyNotification;
    request: ReplyReveal;
    expiresAt: number;
  }>();
  const [activation, setActivation] = useState<{ pageKey: string; event: ReplyNotification }>();
  const projection = useMemo(
    () =>
      projectReplyNotifications({ asks, threads, askUnavailable }, deviceId, { questionThreads }),
    [asks, threads, askUnavailable, deviceId, questionThreads],
  );
  const latest = useRef({ projection, blocked, activate, pageKey, reveal });
  latest.current = { projection, blocked, activate, pageKey, reveal };
  const matches = useCallback((event: ReplyNotification, key: string) => {
    const current = latest.current.projection.candidates.get(event.id);
    return (
      latest.current.pageKey === key &&
      !latest.current.blocked &&
      !!current &&
      current.revision === event.revision &&
      destinationKey(current) === destinationKey(event)
    );
  }, []);
  const current = useCallback(
    (id: string) => {
      const value = latest.current.reveal;
      return (
        !!value &&
        performance.now() < value.expiresAt &&
        value.request.id === id &&
        matches(value.event, value.pageKey)
      );
    },
    [matches],
  );
  useEffect(() => {
    if (!activation) return;
    let frame: number;
    const deadline = performance.now() + REPLY_REVEAL_WAIT_MS;
    const attempt = () => {
      if (!matches(activation.event, activation.pageKey) || performance.now() >= deadline) {
        setActivation(undefined);
        return;
      }
      const event = latest.current.projection.candidates.get(activation.event.id)!;
      if (!modals.current.size && latest.current.activate(event)) {
        setReveal({
          pageKey: activation.pageKey,
          event,
          expiresAt: performance.now() + REPLY_REVEAL_WAIT_MS,
          request: { id: event.id, destination: destinationKey(event), recordKey: event.recordKey },
        });
        setActivation(undefined);
      } else frame = requestAnimationFrame(attempt);
    };
    frame = requestAnimationFrame(attempt);
    return () => cancelAnimationFrame(frame);
  }, [activation, matches]);
  useEffect(() => {
    if (!reveal) return;
    const timeout = setTimeout(
      () => setReveal((value) => (value === reveal ? undefined : value)),
      REPLY_REVEAL_WAIT_MS,
    );
    return () => clearTimeout(timeout);
  }, [reveal]);
  useEffect(() => {
    if (reveal && !matches(reveal.event, reveal.pageKey)) setReveal(undefined);
  }, [projection, blocked, pageKey, reveal, matches]);
  const alreadyReading = useCallback(
    (event: ReplyNotification) =>
      [...readers.current.values()].some(
        (reader) =>
          reader.destination === destinationKey(event) &&
          reader.state.visible &&
          (event.destination === 'Answer' || event.destination === 'Proposal'
            ? reader.state.recordKeys?.includes(event.recordKey)
            : reader.state.atEnd),
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
        previous.state.atEnd === state.atEnd &&
        JSON.stringify(previous.state.recordKeys) === JSON.stringify(state.recordKeys)
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
    (id: string) => setReveal((current) => (current?.request.id === id ? undefined : current)),
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
        if (matches(event, pageKey)) setActivation({ pageKey, event });
      },
    }));
    setScope((current) => (current.observer === scope.observer ? { ...current, events } : current));
  }, [scope, asks, threads, askUnavailable, projection, blocked, alreadyReading, matches, pageKey]);
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
    activating: !!activation && matches(activation.event, activation.pageKey),
    isEligible,
    context: {
      modal,
      reading,
      reveal: reveal && current(reveal.request.id) ? reveal.request : undefined,
      revealed,
      current,
    },
  };
}
