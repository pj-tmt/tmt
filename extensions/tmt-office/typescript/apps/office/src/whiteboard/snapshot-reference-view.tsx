import { snapshotReference } from './snapshot-reference.js';
import { useCopyReference } from '../local/use-copy-reference.js';

export function SnapshotReferenceView({ id }: { id: string }) {
  const reference = snapshotReference(id);
  const { field, message, copy } = useCopyReference(reference);
  return (
    <div className="whiteboard-review-reference">
      <div className="tmt-ui-command">
        <label>
          Local snapshot reference
          <input className="tmt-ui-command-text" ref={field} readOnly value={reference} />
        </label>
        <button className="tmt-ui-command-copy" type="button" onClick={() => void copy()}>
          Copy reference
        </button>
        {message && (
          <p className="tmt-ui-command-feedback" role="status">
            {message}
          </p>
        )}
      </div>
      <p>Read with TMT on this machine. Copying does not send a request.</p>
    </div>
  );
}
