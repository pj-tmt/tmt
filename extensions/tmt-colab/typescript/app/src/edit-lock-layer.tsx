import './edit-lock-layer.css';
import { shareView } from './share-view.js';
import { AgentChip } from './components/agent-chip.js';
import { useCallback, useEffect, useRef, useState, type RefObject } from 'react';
import { activeEditLocks, type LockPresence, type EditLockRecord } from './edit-locks.js';
import type { AgentDestination } from './live-ask.js';
import type { PageView } from './transport.js';
import type { EditLockPosition, mountRenderer } from './renderer.js';
import { toolbarInset } from './toolbar-inset.js';

/** Missing discovery is unknown. A UUID active on any observed machine keeps its lock. */
function holderPresence(agents: readonly AgentDestination[], id: string): LockPresence {
  const rows = agents.filter((agent) => agent.agent === id);
  if (!rows.length) return 'unknown';
  const states = rows.map((agent): LockPresence =>
    agent.presence === 'offline'
      ? 'offline'
      : agent.presence === 'active' && agent.activity?.state === 'ended'
        ? 'ended'
        : (agent.presence ?? 'unknown'),
  );
  if (states.includes('active')) return 'active';
  return states.includes('unknown') ? 'unknown' : 'offline';
}

/** Paint belongs to the opaque renderer; labels stay in this admitted parent overlay. */
export function useEditLockLayer({
  page,
  view,
  agents,
  renderer,
  host,
  toolbar,
  state,
  contentRevision,
  admitted,
}: {
  page: string;
  view: Pick<PageView, 'editLocks'> | undefined;
  agents: readonly AgentDestination[];
  renderer: RefObject<Awaited<ReturnType<typeof mountRenderer>> | null>;
  host: RefObject<HTMLDivElement | null>;
  toolbar: RefObject<HTMLElement | null>;
  state: string;
  contentRevision: number;
  admitted: boolean;
}) {
  const [located, setLocated] = useState<{
    page: string;
    positions: EditLockPosition[];
    checked: boolean;
  }>();
  const [bounds, setBounds] = useState({ left: 0, top: 0, width: 0, height: 0, floating: 8 });
  const onEditLocks = useCallback(
    (positions: EditLockPosition[], checked = false) =>
      setLocated((old) => shareView(old, { page, positions, checked })),
    [page],
  );
  const held = useRef<EditLockRecord[]>([]);
  held.current = shareView(
    held.current,
    admitted ? activeEditLocks(view?.editLocks ?? [], (id) => holderPresence(agents, id)) : [],
  );
  const locks = held.current;
  useEffect(() => {
    if (state === 'ready')
      renderer.current?.editLocks(locks.map((lock) => ({ id: lock.lockId, target: lock.target })));
  }, [renderer, state, contentRevision, locks]);
  useEffect(() => {
    const measure = () => {
      const node = host.current;
      if (!node) return;
      const next = {
        left: node.offsetLeft,
        top: node.offsetTop,
        width: node.clientWidth,
        height: node.clientHeight,
        floating:
          node.offsetTop +
          Math.max(8, toolbarInset(toolbar.current) + 8 - node.getBoundingClientRect().top),
      };
      setBounds((old) =>
        Object.keys(next).every(
          (key) => old[key as keyof typeof old] === next[key as keyof typeof next],
        )
          ? old
          : next,
      );
    };
    measure();
    window.addEventListener('scroll', measure, { passive: true });
    window.addEventListener('resize', measure);
    const observer = new ResizeObserver(measure);
    if (host.current) observer.observe(host.current);
    return () => {
      window.removeEventListener('scroll', measure);
      window.removeEventListener('resize', measure);
      observer.disconnect();
    };
  }, [host, toolbar, page, state]);
  const positions = located?.page === page ? located.positions : [];
  let floating = 0;
  const layer = state === 'ready' && locks.length > 0 && (
    <div className="edit-lock-layer" aria-label="Agent edits">
      {locks.map((lock) => {
        const position = positions.find((row) => row.id === lock.lockId);
        if (
          position &&
          (position.rect.y + position.rect.height <= 0 || position.rect.y >= bounds.height)
        )
          return null;
        // At the top edge a chip cannot fit above the quote; keep the text readable.
        const above = position && position.rect.y >= 32 ? position : undefined;
        const left = above ? Math.max(8, Math.min(above.rect.x, bounds.width - 120)) : 8;
        const label = `${lock.holder.name} is editing`;
        return (
          <span
            key={lock.lockId}
            data-testid="edit-lock"
            data-anchor={
              position ? 'range' : located?.page === page && located.checked ? 'page' : 'pending'
            }
            className="edit-lock-chip"
            role="status"
            title={label}
            style={{
              left: above ? bounds.left + left : undefined,
              right: above ? undefined : 8,
              top: above ? bounds.top + above.rect.y - 4 : bounds.floating + floating++ * 30,
              maxWidth: Math.max(64, bounds.width - left - 8),
              transform: above ? 'translateY(-100%)' : undefined,
            }}
          >
            <AgentChip name={lock.holder.name} activity="unknown" /> is editing
          </span>
        );
      })}
    </div>
  );
  return { onEditLocks, layer };
}
