import { createContext } from 'react';
import type { CommentView, DiscussionRef } from './thread-records.js';

/**
 * A sent message shown at once in its final position, before its verified record exists.
 * Display only: never counted, exported or treated as verified. The composer that sent it
 * owns the draft; the host only shows the echo until the verified record replaces it.
 */
export type MessageEcho = {
  local: string;
  body: string;
  at: number;
  /** The recorded message, once the write returned; its verified record replaces the echo. */
  message?: DiscussionRef;
  /** Present when the send failed before it was recorded. */
  retry?(): void;
};

/**
 * Provided by a host that shows sent replies and Chat messages at once (a thread window, Chat).
 * A composer inside it reports its echo here; without a host it keeps today's send flow.
 */
export const MessageEchoContext = createContext<
  ((echo: MessageEcho | undefined) => void) | undefined
>(undefined);

/** A new echo for a message the composer is sending now. */
export function messageEcho(body: string): MessageEcho {
  return { local: crypto.randomUUID(), body, at: Date.now() };
}

export function echoLanded(echo: MessageEcho, comments: readonly CommentView[]) {
  const message = echo.message;
  return (
    !!message &&
    comments.some(
      (comment) => comment.ref.writer === message.writer && comment.messageId === message.id,
    )
  );
}
