import { createContext } from 'react';

/**
 * App-local presentation only; conversation, draft and admission owners stay with callers. A
 * `discussion` is a thread shown as the Comments drawer's Discussion view.
 */
export const ConversationPresentation = createContext<'chat' | 'thread' | 'discussion'>('thread');
