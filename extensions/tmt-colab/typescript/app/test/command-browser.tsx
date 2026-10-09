/** Deterministic management presentation; no credentials, storage or network effects. */
import 'virtual:tokens.css';
import '@tmt/browser-ui/static.css';
import '../src/style.css';
import { createRoot } from 'react-dom/client';
import { ShareDialog } from '../src/share-dialog.js';
import type { ManagementPort, ManagementView } from '../src/management.js';

const pageId = '11111111-1111-4111-8111-111111111111';
const view: ManagementView = {
  page: {
    pageId,
    sharing: 'link',
    history: 'shared',
    archived: false,
    epoch: '1',
    retentionDays: 30,
    deleted: false,
    lastUpdateAtMs: null,
    expiresAtMs: null,
    warnings: [],
  },
  revision: '1',
  ownerMember: 'owner',
  members: [],
  links: [],
};
const port: ManagementPort = {
  read: async () => view,
  prepare: async (_, selection) => ({
    body: 'fixture-only',
    id: 'operation',
    page: pageId,
    context: view.page,
    operation: selection.operation,
    expiresAt: Date.now() + 60000,
    expectedRevision: '1',
    artifact: { linkId: '22222222-2222-4222-8222-222222222222', seed: 'a'.repeat(43) },
  }),
  send: async () => ({
    operationId: 'operation',
    membershipHead: { revision: '2', statementHash: 'fixture' },
  }),
  verify: async () => view,
};
createRoot(document.getElementById('root')!).render(
  <ShareDialog
    port={port}
    pageId={pageId}
    title="Review notes"
    close={() => {}}
    committed={() => {}}
  />,
);

import { BrowserCommand } from '@tmt/browser-ui/react';
import { useState } from 'react';
export const longCommand = `tmt result "<café>"\n# ${'x'.repeat(256)}`;
function LongCommand() {
  const [feedback, setFeedback] = useState('');
  return (
    <BrowserCommand
      text={longCommand}
      feedback={feedback}
      action={
        <button
          type="button"
          className="tmt-ui-command-copy"
          onClick={async () => {
            await navigator.clipboard.writeText(longCommand);
            setFeedback('Copied.');
          }}
        >
          Copy
        </button>
      }
    />
  );
}
export function mountLong() {
  for (const dialog of document.querySelectorAll('dialog[open]'))
    (dialog as HTMLDialogElement).close();
  document.getElementById('root')!.hidden = true;
  const host = document.createElement('div');
  document.body.append(host);
  createRoot(host).render(<LongCommand />);
}
