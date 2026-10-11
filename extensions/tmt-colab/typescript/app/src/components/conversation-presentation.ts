import { createContext } from 'react';

/** App-local presentation only; conversation, draft and admission owners stay with callers. */
export const ConversationPresentation = createContext<'chat' | 'thread'>('thread');
