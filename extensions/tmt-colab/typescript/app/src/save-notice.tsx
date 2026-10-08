import { Diamond, X } from 'lucide-react';
import { BrowserNotice } from '@tmt/browser-ui/react';
import { text } from './strings.js';

/** What the editor tells the person about a save that did not succeed. The words are the state:
 * "Not saved" for a refusal, "Unconfirmed" when the outcome is not known. */
export interface SaveProblem {
  message: string;
  unconfirmed: boolean;
}
export function SaveNotice({ problem }: { problem: SaveProblem }) {
  return (
    <div className="source-notice">
      <BrowserNotice
        tone={problem.unconfirmed ? 'waiting' : 'blocked'}
        announcement="alert"
        stateLabel={problem.unconfirmed ? text.saveUnconfirmed : text.saveNotSaved}
        title={problem.message}
        mark={problem.unconfirmed ? <Diamond fill="currentColor" /> : <X />}
        body={null}
      />
    </div>
  );
}
