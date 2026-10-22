import { moduleMount } from '@tmt/browser-ui/shell';
import { publicEntry } from './entry.js';
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { RouterProvider } from '@tanstack/react-router';
import { createAppRouter } from './router.js';
import { previewTransport } from './local-pages.js';
import '@tmt/browser-ui/static.css';
import './style.css';

import { MountedApp } from './mounted-app.js';
import { probeCapabilities } from '@tmt/colab-client';
import { UnsupportedBrowserNotice } from './notice-card.js';
import { isHostedOrigin } from './hosted-location.js';
import { startHosted } from './hosted-main.js';
import { startConnectGuide } from './connect-guide.js';

document.body.classList.remove('guidance');
const root = createRoot(document.getElementById('root')!);
async function start() {
  if (isHostedOrigin(location.origin)) {
    if (location.pathname === '/pair') {
      const link = location.href;
      history.replaceState(history.state, '', location.pathname + location.search);
      startConnectGuide(root, link);
      return;
    }
    await startHosted(root);
    return;
  }
  try {
    await probeCapabilities();
  } catch {
    root.render(<UnsupportedBrowserNotice />);
    return;
  }
  // Colab's mounted document moved into the one TMT app at `/colab/` (#3937). An old link keeps
  // its query and fragment (owner `path=`, reading, anchor), which the app maps; a session
  // hand-off (`?tmt-session=`) still opens here.
  if (moduleMount(location.pathname) === 'colab' && !location.search.includes('tmt-session=')) {
    location.replace(`/colab/${location.search}${location.hash}`);
    return;
  }
  if (!location.pathname.startsWith('/r/') && !publicEntry()) {
    root.render(
      <StrictMode>
        <RouterProvider router={createAppRouter(previewTransport)} />
      </StrictMode>,
    );
    return;
  }
  root.render(
    <StrictMode>
      <MountedApp />
    </StrictMode>,
  );
}
void start();
