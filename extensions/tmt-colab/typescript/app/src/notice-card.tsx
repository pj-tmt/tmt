import { Circle, Diamond, LoaderCircle, X } from 'lucide-react';
import type { ReactNode } from 'react';
import { BrowserNotice } from '@tmt/browser-ui/react';
import './notice-card.css';
import { ColabHeader } from './colab-header.js';
import { text } from './strings.js';

/** Capability refusal happens before either entry opens any space or reader session. */
export function UnsupportedBrowserNotice() {
  return (
    <>
      <ColabHeader title={text.browserUpdate} />
      <main>
        <NoticeCard
          state="blocked"
          stateLabel={text.browserUnsupported}
          eyebrow={text.product}
          title={text.browserUpdate}
          testId="unsupported-browser"
        >
          <p>{text.browserUpdateBody}</p>
        </NoticeCard>
      </main>
    </>
  );
}

/** Colab owns the state and recovery; browser-ui presents the explicit words. */
export function NoticeCard({
  state,
  stateLabel,
  eyebrow,
  title,
  children,
  actions,
  testId,
}: {
  state: 'opening' | 'waiting' | 'inactive' | 'ended' | 'blocked';
  stateLabel?: string;
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
      stateLabel={stateLabel ?? (state === 'blocked' ? 'failed' : state)}
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
