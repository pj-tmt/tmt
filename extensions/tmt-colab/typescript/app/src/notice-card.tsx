import { Circle, Diamond, LoaderCircle, X } from 'lucide-react';
import type { ReactNode } from 'react';
import './notice-card.css';

/** Shared state presentation; the surrounding screen owns recovery actions. */
export function NoticeCard({
  state,
  eyebrow,
  title,
  children,
  actions,
  testId,
}: {
  state: 'opening' | 'waiting' | 'inactive' | 'ended' | 'blocked';
  eyebrow: string;
  title: string;
  children?: ReactNode;
  actions?: ReactNode;
  testId?: string;
}) {
  const waiting = state === 'opening' || state === 'waiting';
  const muted = state === 'inactive' || state === 'ended';
  return (
    <section
      className={`notice ${waiting ? 'waiting' : muted ? 'ended' : 'blocked'}`}
      role={waiting || state === 'inactive' ? 'status' : 'alert'}
      data-testid={testId}
    >
      <span className="notice-mark" aria-hidden>
        {state === 'opening' ? (
          <LoaderCircle />
        ) : state === 'waiting' ? (
          <Diamond fill="currentColor" />
        ) : muted ? (
          <Circle />
        ) : (
          <X />
        )}
      </span>
      <p className="notice-eyebrow">{eyebrow}</p>
      <h2>{title}</h2>
      {children}
      {actions && <div className="notice-actions">{actions}</div>}
    </section>
  );
}
