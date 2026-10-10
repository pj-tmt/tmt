import { useLayoutEffect, useRef } from 'react';
import type { PageAsk } from './ask-panel.js';
import { text } from './strings.js';

function identity(record: PageAsk) {
  return JSON.stringify([record.writer, record.operationId, record.requestId, record.reply]);
}

/** One read-only page owner, independent of mirrored or reopened conversation views. */
export function ReplyAnnouncer({
  initialRecords,
  records,
}: {
  initialRecords?: readonly PageAsk[];
  records?: readonly PageAsk[];
}) {
  const observe = useRef<(records: readonly PageAsk[] | undefined) => void>(undefined);
  useLayoutEffect(() => {
    const live = document.createElement('div');
    live.className = 'reply-announcement';
    live.setAttribute('role', 'status');
    live.setAttribute('aria-live', 'polite');
    live.setAttribute('aria-atomic', 'true');
    const relocate = () => {
      // A native modal makes the rest of the page inert, including its live regions.
      const target = [...document.querySelectorAll('dialog:modal')].at(-1) ?? document.body;
      if (live.parentElement === target) return;
      live.textContent = '';
      target.append(live);
    };
    relocate();
    const modal = new MutationObserver(relocate);
    modal.observe(document.body, {
      subtree: true,
      childList: true,
      attributes: true,
      attributeFilter: ['open'],
    });
    let seen: Set<string> | undefined;
    const baseline = (values: readonly PageAsk[]) =>
      new Set(values.filter((record) => record.reply !== undefined).map(identity));
    if (initialRecords !== undefined) seen = baseline(initialRecords);
    let frame: number | undefined;
    const pending: string[] = [];
    let mounted = false;
    observe.current = (values) => {
      // Page changes can briefly render the previous view before the loader reset.
      if (!mounted) {
        mounted = true;
        return;
      }
      if (values === undefined) return;
      if (!seen) {
        seen = baseline(values);
        return;
      }
      for (const record of values) {
        if (record.reply === undefined || seen.has(identity(record))) continue;
        seen.add(identity(record));
        pending.push(text.askReplied(record.agentName || text.askAgentLabel));
      }
      if (!pending.length || frame !== undefined) return;
      live.textContent = '';
      frame = requestAnimationFrame(() => {
        frame = undefined;
        relocate();
        live.textContent = pending.splice(0).join(' ');
      });
    };
    return () => {
      observe.current = undefined;
      modal.disconnect();
      if (frame !== undefined) cancelAnimationFrame(frame);
      live.remove();
    };
  }, [initialRecords]);
  useLayoutEffect(() => observe.current?.(records), [records, initialRecords]);
  return null;
}
