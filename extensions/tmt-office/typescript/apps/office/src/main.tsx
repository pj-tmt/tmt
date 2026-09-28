import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { OfficeApp } from './office-app.js';
import { createOfficeRouter } from './router.js';
import './styles.css';
import type { OfficeRuntime } from './auth/firebase-session.js';
import type { LocalRuntime } from './local/local-runtime.js';

const root = document.getElementById('root');
if (!root) throw new Error('Office root element is missing.');

const app = createRoot(root);
let runtime: OfficeRuntime | undefined;
let local: LocalRuntime | undefined;
let active = true;
import.meta.hot?.dispose(() => {
  active = false;
  app.unmount();
  void runtime?.dispose();
  local?.dispose();
});

async function mount(): Promise<void> {
  try {
    // Vite removes this branch (and the SDK chunk) from the default build.
    if (import.meta.env.MODE === 'emulator' || import.meta.env.MODE === 'cloud') {
      const { startOfficeRuntime } = await import('./auth/firebase-session.js');
      if (!active) return;
      runtime = startOfficeRuntime(import.meta.env.MODE, location.hostname, import.meta.env);
    } else if (import.meta.env.MODE === 'offline') {
      const { startLocalRuntime } = await import('./local/local-runtime.js');
      local = startLocalRuntime(location);
    }
  } catch {
    if (active)
      app.render(
        <p role="alert">
          {import.meta.env.MODE === 'offline'
            ? 'Local Office could not initialize. Rerun tmt office start and reopen its URL.'
            : 'Office could not initialize. Check the selected environment and Firebase configuration, then reload.'}
        </p>
      );
    return;
  }
  app.render(
    <StrictMode>
      <OfficeApp
        router={createOfficeRouter()}
        session={runtime?.session}
        worlds={runtime?.worlds}
        blocks={runtime?.blocks}
        pairing={runtime?.pairing}
        spaces={runtime?.spaces}
        mode={runtime?.mode}
        local={local}
      />
    </StrictMode>
  );
}

void mount();
