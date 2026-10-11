import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { Notifications } from './notifications.js';
createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <Notifications />
  </StrictMode>,
);
