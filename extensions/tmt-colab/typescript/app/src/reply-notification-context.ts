import { createContext, useCallback, useContext, useId, useLayoutEffect } from 'react';

export interface ConversationReading {
  visible: boolean;
  atEnd: boolean;
  recordKeys?: readonly string[];
}
export interface ReplyReveal {
  id: string;
  destination: string;
  recordKey: string;
}

/** Page-local presentation signals from the existing surface and scroll owners. */
export const ReplyNotificationContext = createContext<{
  blockSurface?: 'page' | 'comments' | 'covered';
  modal(id: string, open: boolean): void;
  reading(id: string, destination: string, state: ConversationReading | null): void;
  reveal?: ReplyReveal;
  revealed(id: string): void;
  current(id: string): boolean;
} | null>(null);

export function useNotificationModalPresence() {
  const modal = useContext(ReplyNotificationContext)?.modal;
  const id = useId();
  const report = useCallback((open: boolean) => modal?.(id, open), [modal, id]);
  useLayoutEffect(() => () => report(false), [report]);
  return report;
}

export function useNotificationConversation(destination: string | undefined) {
  const owner = useContext(ReplyNotificationContext);
  const reading = owner?.reading;
  const id = useId();
  const report = useCallback(
    (state: ConversationReading | null) => {
      if (destination) reading?.(id, destination, state);
    },
    [reading, id, destination],
  );
  useLayoutEffect(() => () => report(null), [report]);
  const request = owner?.reveal;
  return {
    report,
    blockSurface: owner?.blockSurface ?? 'page',
    reveal: request?.destination === destination ? request : undefined,
    revealed: owner?.revealed,
    current: owner?.current,
  };
}

export const threadNotificationDestination = (writer: string, id: string) =>
  JSON.stringify(['Thread', writer, id]);

export const answerNotificationDestination = (id: string) => JSON.stringify(['Answer', id]);
export const proposalNotificationDestination = (writer: string, id: string) =>
  JSON.stringify(['Proposal', writer, id]);
