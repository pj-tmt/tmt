import { publicEntry } from './entry.js';
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { RouterProvider } from '@tanstack/react-router';
import { createAppRouter } from './router.js';
import { previewTransport } from './local-pages.js';
import '@tmt/browser-ui/static.css';
import './style.css';

import { MountedApp } from './mounted-app.js';

document.body.classList.remove('guidance');
const root = createRoot(document.getElementById('root')!);
async function start() {
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
