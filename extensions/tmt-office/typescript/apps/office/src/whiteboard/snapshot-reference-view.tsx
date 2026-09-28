import { snapshotReference } from './snapshot-reference.js';
import { useCopyReference } from '../local/use-copy-reference.js';

export function SnapshotReferenceView({ id }: { id: string }) {
  const reference = snapshotReference(id);
  const { field, message, copy } = useCopyReference(reference);
  return (
    <div className="whiteboard-review-reference">
      <label>
        Local snapshot reference
        <input ref={field} readOnly value={reference} />
      </label>
      <button onClick={() => void copy()}>Copy reference</button>
      {message && <p role="status">{message}</p>}
      <p>Read with TMT on this machine. Copying does not send a request.</p>
    </div>
  );
}
