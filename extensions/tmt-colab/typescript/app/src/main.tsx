import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { RouterProvider } from '@tanstack/react-router';
import { createAppRouter } from './router.js';
import { previewTransport } from './local-pages.js';
import 'virtual:tokens.css';
import './style.css';

import { mountedTransport } from './mounted.js';
import { text } from './strings.js';

const root = createRoot(document.getElementById('root')!);
async function start() {
  if (!location.pathname.startsWith('/r/')) {
    root.render(
      <StrictMode>
        <RouterProvider router={createAppRouter(previewTransport)} />
      </StrictMode>,
    );
    return;
  }
  root.render(
    <main className="notice" role="status">
      <h1>{text.registering}</h1>
    </main>,
  );
  try {
    const { space, transport } = await mountedTransport();
    root.render(
      <StrictMode>
        <RouterProvider router={createAppRouter(transport, space)} />
      </StrictMode>,
    );
  } catch {
    root.render(
      <main className="notice" role="alert">
        <h1>{text.registrationFailed}</h1>
        <button onClick={() => location.reload()}>{text.reload}</button>
      </main>,
    );
  }
}
void start();
