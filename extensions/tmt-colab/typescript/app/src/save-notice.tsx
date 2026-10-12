import type { ReactNode } from 'react';
import { Diamond, X } from 'lucide-react';
import { BrowserNotice } from '@tmt/browser-ui/react';
import { text } from './strings.js';

/** What the editor tells the person about a save that did not succeed. The words are the state:
 * "Not saved" for a refusal, "Unconfirmed" when the outcome is not known. A full page offers the
 * way out that can help: `compact` when combining changes frees room, `export` when it cannot,
 * and `compacted` once it did, so the person saves again by choice. */
export interface SaveProblem {
  message: string;
  unconfirmed: boolean;
  full?: 'compact' | 'export' | 'compacted';
}
export function SaveNotice({ problem, actions }: { problem: SaveProblem; actions?: ReactNode }) {
  const waiting = problem.unconfirmed || problem.full === 'compacted';
  return (
    <div className="source-notice">
      <BrowserNotice
        tone={waiting ? 'waiting' : 'blocked'}
        announcement={problem.full === 'compacted' ? 'status' : 'alert'}
        stateLabel={
          problem.full === 'compacted'
            ? text.pageSize.stateCompacted
            : problem.unconfirmed
              ? text.saveUnconfirmed
              : text.saveNotSaved
        }
        title={problem.message}
        mark={waiting ? <Diamond fill="currentColor" /> : <X />}
        body={null}
        actions={actions}
      />
    </div>
  );
}
