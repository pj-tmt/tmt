import { useState } from 'react';
import { browserUiClasses as c } from '@tmt/browser-ui/static';

/** Presentation of the already verified artifact; copying never creates or stores a link. */
export function BearerArtifact({ url }: { url: string }) {
  const [feedback, setFeedback] = useState('');
  return (
    <div className={c.command}>
      <div className="bearer-values">
        <label>
          Share link
          <input className={c.commandText} readOnly value={url} />
        </label>
      </div>
      <button
        className={c.commandCopy}
        type="button"
        onClick={async () => {
          try {
            await navigator.clipboard.writeText(url);
            setFeedback('Copied.');
          } catch {
            setFeedback('Clipboard unavailable. Select and copy the displayed link.');
          }
        }}
      >
        Copy share link
      </button>
      {feedback && (
        <p className={c.commandFeedback} role="status">
          {feedback}
        </p>
      )}
    </div>
  );
}
