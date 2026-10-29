import { useEffect, useMemo, useState } from 'react';
import type { PageHeaderAction } from './page-header-actions.js';

/** A request for the Chat composer to take focus once the panel is open. */
export interface ChatActivation {
  id: string;
  insertMention: boolean;
}

/**
 * Which of the page's panels is open (at most one), whether Chat has been opened on this page, the
 * Chat activation waiting for its composer, and the thread the Comments panel has selected. The
 * header, the keys, a notification and a thread link all open or close panels, so they say what they
 * mean here (`show`, `showThread`, `openChat`, `clear`) instead of setting the pieces. Everything
 * clears when the page changes, and a pending activation when Chat closes or the binding changes.
 */
export function usePagePanels(pageId: string, binding: unknown) {
  const [panel, setPanel] = useState<PageHeaderAction | null>(null);
  // Closing keeps the last pane intact through the host's exit motion.
  const [conversation, setConversation] = useState<'chat' | 'comments'>('comments');
  const [chatOpened, setChatOpened] = useState(false);
  const [chatActivation, setChatActivation] = useState<ChatActivation>();
  const [activeThread, setActiveThread] = useState<string | null>(null);
  useEffect(() => {
    if (panel !== 'chat') setChatActivation(undefined);
  }, [panel]);
  useEffect(() => setChatActivation(undefined), [binding]);
  useEffect(() => {
    setActiveThread(null);
    setPanel(null);
    setChatOpened(false);
    setConversation('comments');
    setChatActivation(undefined);
  }, [pageId]);

  const actions = useMemo(
    () => ({
      /** The header opens a panel, or closes it when it is the one open. */
      toggle(value: PageHeaderAction) {
        if (value === 'chat' || value === 'comments') setConversation(value);
        if (value === 'chat') setChatOpened(true);
        setPanel((previous) => (previous === value ? null : value));
      },
      show(value: PageHeaderAction) {
        if (value === 'chat' || value === 'comments') setConversation(value);
        setPanel(value);
      },
      close() {
        setPanel(null);
      },
      /** Closes the open panel when it is one of `these`. */
      closeIf(...these: PageHeaderAction[]) {
        setPanel((current) => (current && these.includes(current) ? null : current));
      },
      /** No panel and no selected thread. */
      clear() {
        setActiveThread(null);
        setPanel(null);
      },
      /** Comments, with this thread selected. */
      showThread(id: string) {
        setConversation('comments');
        setActiveThread(id);
        setPanel('comments');
      },
      /** Chat, with `activation` for its composer (none: it opens without taking focus). */
      openChat(activation?: ChatActivation) {
        setConversation('chat');
        setChatOpened(true);
        setPanel('chat');
        setChatActivation(activation);
      },
      /** The composer took its activation. */
      settleChatActivation(id: string) {
        setChatActivation((current) => (current?.id === id ? undefined : current));
      },
      setActiveThread,
    }),
    [],
  );
  return { panel, conversation, chatOpened, chatActivation, activeThread, ...actions };
}
