/** The design gallery's hosted-shell frame, mounted on its own page for a reload check. */
import { createRoot } from 'react-dom/client';
import { frameFor } from '@tmt/browser-ui/gallery/frames';

export function mountHostedFrame() {
  const host = document.createElement('div');
  host.id = 'hosted-fixture';
  document.body.replaceChildren(host);
  createRoot(host).render(frameFor('hosted', new URLSearchParams('frame=hosted')));
}
