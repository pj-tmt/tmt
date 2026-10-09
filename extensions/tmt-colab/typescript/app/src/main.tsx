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

const root = createRoot(document.getElementById('root')!);
async function start() {
  try {
    await probeCapabilities();
  } catch {
    root.render(<UnsupportedBrowserNotice />);
    return;
  }
  if (!location.pathname.startsWith('/r/')) {
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
