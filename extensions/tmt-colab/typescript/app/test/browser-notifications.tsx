import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { Notifications } from '../../../../../design/browser-ui/test/fixtures/notifications.js';
export function mount() {
  document.getElementById('root')?.setAttribute('hidden', '');
  // The product's fixed header reserves body padding; this fixture has its own sticky header.
  document.body.style.paddingTop = '0';
  const host = document.createElement('main');
  document.body.append(host);
  createRoot(host).render(
    <StrictMode>
      <Notifications />
    </StrictMode>,
  );
}
