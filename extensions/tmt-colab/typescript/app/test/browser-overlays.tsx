import { createRoot } from 'react-dom/client';
import { Overlays } from '@tmt/browser-ui/test-fixtures/overlays.tsx';
export function mount() {
  document.getElementById('root')?.setAttribute('hidden', '');
  const host = document.createElement('main');
  document.body.append(host);
  createRoot(host).render(<Overlays />);
}
