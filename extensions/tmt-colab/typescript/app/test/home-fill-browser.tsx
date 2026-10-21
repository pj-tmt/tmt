import { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { useSummaryFill } from '../src/home-fill.js';

declare global {
  interface Window {
    /** Pages still without a summary; the spec sets it the way the owner's fold would. */
    fillMissing: number;
    /** List reads made by the hook. */
    fillReads: number;
  }
}

/** The list's re-read: counts the reads and shows the pages missing at that moment. */
function Fixture() {
  const [missing, setMissing] = useState(window.fillMissing);
  const [mounted, setMounted] = useState(true);
  return (
    <main id="fill-fixture">
      <button type="button" onClick={() => setMissing(window.fillMissing)}>
        Sync
      </button>
      <button type="button" onClick={() => setMounted(false)}>
        Leave
      </button>
      {mounted ? (
        <Reader
          missing={missing}
          reread={() => {
            window.fillReads += 1;
            // The next read sees what the fold has finished by now.
            setMissing(window.fillMissing);
          }}
        />
      ) : (
        <p id="left">Left the list</p>
      )}
    </main>
  );
}

function Reader({ missing, reread }: { missing: number; reread: () => void }) {
  const reading = useSummaryFill(missing, reread);
  return (
    <p id="state" data-reading={String(reading)}>
      {missing} missing
    </p>
  );
}

export function mount(missing: number) {
  window.fillMissing = missing;
  window.fillReads = 0;
  document.getElementById('root')?.setAttribute('hidden', '');
  const host = document.createElement('div');
  document.body.append(host);
  createRoot(host).render(<Fixture />);
}
