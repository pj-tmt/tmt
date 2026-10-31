import { createContext } from 'react';
import type { PageAsk } from './ask-panel.js';
import { canTransition, type LedgerState } from './ask-records.js';
import type { CommentView, DiscussionRef } from './thread-records.js';

export type EchoAttachment = {
  id: string;
  filename: string;
  mediaType: string;
  size: number;
  /** Bounded picked bytes for display only; never used by the admitted read owner. */
  preview: Promise<string | undefined>;
};

/**
 * A sent message shown at once in its final position, before its verified record exists.
 * Display only: never counted, exported or treated as verified. The composer that sent it
 * owns the draft; the host only shows the echo until the verified record replaces it.
 */
export type MessageEcho = {
  local: string;
  body: string;
  at: number;
  attachments: readonly EchoAttachment[];
  /** Captured display labels reserve the final recipient-status footprint from the first frame. */
  recipients: readonly string[];
  sending: boolean;
  /** Outcomes of this explicit send; only admitted records may replace its pending turn. */
  deliveries: readonly { operationId: string; state: LedgerState }[];
  /** The recorded message, once the write returned; its verified record replaces the echo. */
  message?: DiscussionRef;
  /** Present when the send failed before it was recorded: the row then holds the draft. */
  failure?: { reference: string; retry(): void; edit(): void };
};

/**
 * Provided by a host that shows sent replies and Chat messages at once (a thread window, Chat).
 * A composer inside it reports its echo here; without a host it keeps today's send flow.
 */
export type MessageEchoHost = {
  show(echo: MessageEcho): void;
  remove(local: string): void;
};
export const MessageEchoContext = createContext<MessageEchoHost | undefined>(undefined);

/** A new echo for a message the composer is sending now. */
export function messageEcho(
  body: string,
  attachments: readonly EchoAttachment[] = [],
  recipients: readonly string[] = [],
): MessageEcho {
  return {
    local: crypto.randomUUID(),
    body,
    at: Date.now(),
    attachments,
    recipients,
    sending: true,
    deliveries: [],
  };
}

export function echoLanded(
  echo: MessageEcho,
  comments: readonly CommentView[],
  asks: readonly PageAsk[],
) {
  const message = echo.message;
  return (
    !echo.sending &&
    echo.deliveries.every((delivery) =>
      asks.some(
        (ask) =>
          ask.writer === message?.writer &&
          ask.operationId === delivery.operationId &&
          (ask.state === delivery.state || canTransition(delivery.state, ask.state)),
      ),
    ) &&
    !!message &&
    comments.some(
      (comment) => comment.ref.writer === message.writer && comment.messageId === message.id,
    )
  );
}
