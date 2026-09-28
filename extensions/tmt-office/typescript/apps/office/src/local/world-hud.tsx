import { Link } from '@tanstack/react-router';
import type { ReactNode } from 'react';

/** Shared presentation only: each page retains its actions, selection and draft. */
export function WorldHud({ children }: { children: ReactNode }) {
  return (
    <header className="world-hud" role="region" aria-label="Office controls">
      <Link className="brand" to="/local">
        tmt <span>office</span>
      </Link>
      <div className="world-hud-actions">{children}</div>
      <details
        className="world-connection"
        onKeyDown={(event) => {
          if (event.key === 'Escape' && event.currentTarget.open) {
            event.stopPropagation();
            event.currentTarget.open = false;
            event.currentTarget.querySelector('summary')?.focus();
          }
        }}
      >
        <summary aria-label="Local office details">Local</summary>
        <p>
          Stored on this computer. This private loopback view can edit Office data and send
          explicitly confirmed requests or announcements to TMT inboxes. It does not connect to a
          remote world.
        </p>
      </details>
    </header>
  );
}
