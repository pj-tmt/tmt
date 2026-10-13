import { createRoot } from 'react-dom/client';
import { DialogFixture } from '../../../../../design/browser-ui/test/fixtures/dialog.js';
import '../src/style.css';
export function render() {
  const root = createRoot(document.getElementById('root')!);
  root.render(<DialogFixture />);
}
