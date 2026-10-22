import { createRoot } from 'react-dom/client';
import { DialogFixture } from '@tmt/browser-ui/test-fixtures/dialog.tsx';
import '../src/style.css';
export function render() {
  const root = createRoot(document.getElementById('root')!);
  root.render(<DialogFixture />);
}
