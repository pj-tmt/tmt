import { useState } from 'react';
import { Plus } from 'lucide-react';
import { BrowserAction, BusyMark } from '../src/react.js';

/** The host owns pending work and submission. The named SVG exercises the decorative-slot
 * accessibility boundary: even host-supplied icon metadata must not repeat the label. */
export function ActionExample() {
  const [busy, setBusy] = useState(false);
  const [requests, setRequests] = useState(0);
  const [submits, setSubmits] = useState(0);
  return (
    <div data-action-example>
      <form
        onSubmit={(event) => {
          event.preventDefault();
          setSubmits((n) => n + 1);
        }}
      >
        <div className="gallery-row">
          <BrowserAction type="button" variant="secondary" label="Rename" onActivate={() => {}} />
          <BrowserAction type="button" variant="ghost" label="Delete" onActivate={() => {}} />
          <BrowserAction
            type="submit"
            variant="primary"
            label="New project"
            leadingIcon={<Plus role="img" aria-label="Decorative add" aria-hidden={false} />}
            busyMark={<BusyMark />}
            busy={busy}
            onActivate={() => {
              setRequests((n) => n + 1);
              setBusy(true);
            }}
          />
        </div>
        <p role="status">
          Requests {requests} · Submits {submits}
        </p>
      </form>
      <BrowserAction
        type="button"
        variant="secondary"
        label="Finish example"
        onActivate={() => setBusy(false)}
      />
    </div>
  );
}
