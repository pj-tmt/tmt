import { useLayoutEffect, useRef, type RefObject } from 'react';
import { useNotificationConversation } from './reply-notification-context.js';

/** Cards report only presentation geometry; the page observer owns participation. */
export function useNotificationBlock(
  destination: string,
  block: RefObject<HTMLElement | null>,
  open: boolean,
  surface: 'page' | 'comments' = 'page',
) {
  const notification = useNotificationConversation(destination);
  const latest = useRef(notification);
  latest.current = notification;
  const reportNow = useRef<() => void>(undefined);
  const requestNow = useRef<() => void>(undefined);
  const revealFrame = useRef<number>(undefined);
  useLayoutEffect(() => {
    reportNow.current?.();
    requestNow.current?.();
  });
  useLayoutEffect(() => {
    const node = block.current;
    if (!node) return;
    const report = () => {
      const rect = node.getBoundingClientRect();
      const header = latest.current.headerBottom?.() ?? 0;
      const visible =
        open &&
        surface === latest.current.blockSurface &&
        !!node.getClientRects().length &&
        !node.closest('[hidden]') &&
        !node.closest('[data-detached="true"]') &&
        !document.hidden &&
        rect.bottom > header &&
        rect.top < window.innerHeight;
      const recordKeys = visible
        ? [...node.querySelectorAll<HTMLElement>('[data-conversation-record]')]
            .filter((record) => {
              const bounds = record.getBoundingClientRect();
              return (
                bounds.bottom > header &&
                bounds.top < window.innerHeight &&
                !!record.getClientRects().length
              );
            })
            .map((record) => record.dataset.conversationRecord!)
        : [];
      latest.current.report({ visible, atEnd: false, recordKeys });
      requestNow.current?.();
    };
    reportNow.current = report;
    const clear = () => node.removeAttribute('data-arrived');
    const resize = new ResizeObserver(report);
    resize.observe(node);
    window.addEventListener('scroll', report, true);
    window.addEventListener('resize', report);
    document.addEventListener('visibilitychange', report);
    window.addEventListener('pointerdown', clear, true);
    window.addEventListener('keydown', clear, true);
    node.addEventListener('blur', clear, true);
    report();
    return () => {
      reportNow.current = undefined;
      resize.disconnect();
      window.removeEventListener('scroll', report, true);
      window.removeEventListener('resize', report);
      document.removeEventListener('visibilitychange', report);
      window.removeEventListener('pointerdown', clear, true);
      window.removeEventListener('keydown', clear, true);
      node.removeEventListener('blur', clear, true);
      clear();
    };
  }, [destination, open, block, surface, notification.blockSurface]);
  useLayoutEffect(() => {
    const request = notification.reveal;
    if (!request || !open) return;
    const requestFrame = () => {
      if (revealFrame.current !== undefined) return;
      revealFrame.current = requestAnimationFrame(() => {
        revealFrame.current = undefined;
        const node = block.current;
        if (
          !node ||
          surface !== latest.current.blockSurface ||
          !latest.current.current?.(request.id) ||
          node.closest('[hidden]') ||
          node.closest('[data-detached="true"]') ||
          !node.getClientRects().length ||
          ![...node.querySelectorAll<HTMLElement>('[data-conversation-record]')].some(
            (record) => record.dataset.conversationRecord === request.recordKey,
          )
        )
          return;
        node.scrollIntoView({ block: 'start', behavior: 'instant' });
        const header = latest.current.headerBottom?.() ?? 0;
        const top = node.getBoundingClientRect().top;
        if (top < header + 6) window.scrollBy({ top: top - header - 6, behavior: 'instant' });
        node.focus({ preventScroll: true });
        node.setAttribute('data-arrived', '');
        latest.current.revealed?.(request.id);
      });
    };
    requestNow.current = requestFrame;
    requestFrame();
    return () => {
      requestNow.current = undefined;
      if (revealFrame.current !== undefined) cancelAnimationFrame(revealFrame.current);
      revealFrame.current = undefined;
    };
  }, [notification.reveal, open, block, surface, notification.blockSurface]);
}
