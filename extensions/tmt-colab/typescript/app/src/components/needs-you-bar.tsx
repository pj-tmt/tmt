import type { ReactNode } from 'react';
import './needs-you.css';

/** `needs`: the reader owes something (the attention-coloured `◆`). `agent`: the agent owes the
 * next step (a neutral `◇`). `done`: settled, one faded neutral line (`✓`). */
export type NeedsYouTone = 'needs' | 'agent' | 'done';
const marks: Record<NeedsYouTone, string> = { needs: '◆', agent: '◇', done: '✓' };

/** The one line every "needs you" component starts with: a light band, the mark plus a word (never
 * colour alone), who and why in neutral text, and inline text actions on the right. An open line
 * reads as a request (`word · who: why`), a done line as facts (`word · who · why`). */
export function NeedsYouBar({
  tone,
  word,
  who,
  why,
  actions,
  role,
}: {
  tone: NeedsYouTone;
  word: string;
  who?: string;
  why?: string;
  actions?: ReactNode;
  role?: 'status';
}) {
  return (
    <div className="needs-you-bar" data-tone={tone} role={role}>
      <span className="needs-you-line">
        <span className="needs-you-mark" aria-hidden="true">
          {marks[tone]}
        </span>{' '}
        <strong>{word}</strong>
        {who && (
          <>
            {' · '}
            <span className="needs-you-who">{who}</span>
          </>
        )}
        {why && (tone === 'done' || !who ? ` · ${why}` : `: ${why}`)}
      </span>
      {actions && <span className="needs-you-actions">{actions}</span>}
    </div>
  );
}
