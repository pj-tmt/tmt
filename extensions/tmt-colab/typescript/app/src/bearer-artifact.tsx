import { useState } from 'react';
import { browserUiClasses as c } from '@tmt/browser-ui/static';

/** Presentation of the already verified artifact; copying never creates or stores a link. */
export function BearerArtifact({ linkId, seed }: { linkId: string; seed: string }) {
  const [feedback, setFeedback] = useState('');
  return (
    <div className={c.command}>
      <div className="bearer-values">
        <label>
          Link ID
          <input className={c.commandText} readOnly value={linkId} />
        </label>
        <label>
          Link seed
          <input className={c.commandText} readOnly value={seed} />
        </label>
      </div>
      <button
        className={c.commandCopy}
        type="button"
        onClick={async () => {
          try {
            await navigator.clipboard.writeText(`Link ID: ${linkId}\nLink seed: ${seed}`);
            setFeedback('Copied.');
          } catch {
            setFeedback('Clipboard unavailable. Select and copy the displayed values.');
          }
        }}
      >
        Copy link ID and seed
      </button>
      {feedback && (
        <p className={c.commandFeedback} role="status">
          {feedback}
        </p>
      )}
    </div>
  );
}
