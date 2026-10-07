import { Circle, Diamond, LoaderCircle, X } from 'lucide-react';
import type { ReactNode } from 'react';
import { BrowserNotice } from '@tmt/browser-ui/react';
import './notice-card.css';

/** Colab owns the state and recovery; browser-ui presents the explicit words. */
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
  const notice = (
    <BrowserNotice
      tone={waiting ? 'waiting' : muted ? 'muted' : 'blocked'}
      announcement={waiting || state === 'inactive' ? 'status' : 'alert'}
      stateLabel={state === 'blocked' ? 'failed' : state}
      eyebrow={eyebrow}
      title={title}
      mark={
        state === 'opening' ? (
          <LoaderCircle />
        ) : state === 'waiting' ? (
          <Diamond fill="currentColor" />
        ) : muted ? (
          <Circle />
        ) : (
          <X />
        )
      }
      body={children}
      actions={actions}
    />
  );
  return testId ? <div data-testid={testId}>{notice}</div> : notice;
}
