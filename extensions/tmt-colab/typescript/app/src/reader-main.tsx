import { entryMount, publicEntry } from './entry.js';
import { createRoot } from 'react-dom/client';
import type { AttachmentBinding } from './attachment-service.js';
import { ReaderApp, type ReaderState } from './reader-app.js';
import type { ReaderMetadata } from './link-metadata.js';
import type { ReaderEdit } from './reader-source.js';
import { isOlderReaderFragment, parseReaderFragment } from './reader-link.js';
import { accessEnded, ReaderSession } from './reader.js';
import '@tmt/browser-ui/static.css';
import './reader-style.css';
import { probeCapabilities } from '@tmt/colab-client';
import { UnsupportedBrowserNotice } from './notice-card.js';
import { isHostedOrigin } from './hosted-location.js';
import { startHosted } from './hosted-main.js';

/** Public entry for a share link. The fragment holds the seed: it leaves the address bar
 * before anything else runs and is never stored, logged or sent to the server. */
const hosted = isHostedOrigin(location.origin);
let fragment = hosted ? '' : location.hash;
if (!hosted) history.replaceState(history.state, '', location.pathname + location.search);
const root = createRoot(document.getElementById('root')!);
let shown: ReaderState = { kind: 'opening' };
let files: AttachmentBinding | undefined;
let canEdit = false;
let edit: ReaderEdit | undefined;
let metadata: ReaderMetadata | undefined;
const render = () =>
  root.render(
    <ReaderApp
      state={shown}
      attachments={files}
      canEdit={canEdit}
      edit={edit}
      metadata={metadata}
    />,
  );
const show = (state: ReaderState) => {
  shown = state;
  render();
};
async function start() {
  if (hosted) {
    await startHosted(root);
    return;
  }
  try {
    await probeCapabilities();
  } catch {
    root.render(<UnsupportedBrowserNotice />);
    return;
  }
  show({ kind: 'opening' });
  if (
    (!/^\/read\/[A-Za-z0-9_-]{4,64}$/.test(location.pathname) &&
      !/^\/r\/[a-z0-9]+\/x\/colab\/read$/.test(location.pathname)) ||
    location.search
  ) {
    return show({ kind: 'invalid' });
  }
  let link;
  const older = isOlderReaderFragment(fragment);
  try {
    const parsed = parseReaderFragment(fragment);
    fragment = '';
    link = await parsed;
    if (publicEntry() && location.pathname.slice(6) !== link.link)
      throw new Error('Invalid reader target');
  } catch {
    return show({ kind: older ? 'older' : 'invalid' });
  }
  canEdit = link.role === 'editor';
  const failure = (error: Error) => show({ kind: accessEnded(error) ? 'ended' : 'failed' });
  try {
    let session: ReaderSession | undefined;
    const opened = await ReaderSession.open(
      link,
      publicEntry() ? entryMount() : new URL('./', location.href),
      (view) => {
        metadata = session?.metadata;
        show({ kind: 'ready', view, pageId: link.page });
      },
      failure,
    );
    session = opened;
    files = opened.attachments;
    metadata = opened.metadata;
    if (canEdit) edit = (source, base, onWaiting) => opened.edit(source, base, onWaiting);
    render();
    addEventListener('pagehide', () => opened.close());
  } catch (error) {
    failure(error instanceof Error ? error : new Error('Reader unavailable'));
  }
}
void start();
