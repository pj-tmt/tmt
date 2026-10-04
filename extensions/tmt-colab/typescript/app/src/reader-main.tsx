import { createRoot } from 'react-dom/client';
import { ReaderApp, type ReaderState } from './reader-app.js';
import { parseReaderFragment } from './reader-link.js';
import { accessEnded, ReaderSession } from './reader.js';
import 'virtual:tokens.css';
import './style.css';

/** Public entry for a read-only link. The fragment holds the seed: it leaves the address bar
 * before anything else runs and is never stored, logged or sent to the server. */
const fragment = location.hash;
history.replaceState(history.state, '', location.pathname + location.search);
const root = createRoot(document.getElementById('root')!);
const show = (state: ReaderState) => root.render(<ReaderApp state={state} />);
async function start() {
  show({ kind: 'opening' });
  if (!/^\/r\/[a-z0-9]+\/x\/colab\/read$/.test(location.pathname) || location.search) {
    return show({ kind: 'invalid' });
  }
  let link;
  try {
    link = parseReaderFragment(fragment);
  } catch {
    return show({ kind: 'invalid' });
  }
  const failure = (error: Error) => show({ kind: accessEnded(error) ? 'ended' : 'failed' });
  try {
    const session = await ReaderSession.open(
      link,
      new URL('./', location.href),
      (view) => show({ kind: 'ready', view }),
      failure,
    );
    addEventListener('pagehide', () => session.close());
  } catch (error) {
    failure(error instanceof Error ? error : new Error('Reader unavailable'));
  }
}
void start();
