/** Deterministic management presentation; no credentials, storage or network effects. */
import 'virtual:tokens.css';
import '@tmt/browser-ui/static.css';
import '../src/style.css';
import { createRoot } from 'react-dom/client';
import { ShareDialog } from '../src/share-dialog.js';
import type { ManagementPort, ManagementView } from '../src/management.js';

const params = new URLSearchParams(location.search);
const shareCopy = params.has('share-copy');
// With `unconfirmed`, the first verification fails and a second one succeeds.
const unconfirmed = params.has('unconfirmed');
// A deliberately different artifact role proves results do not use the role picker.
const artifactRole = params.get('artifact-role');
let verifications = 0;
const pageId = '11111111-1111-4111-8111-111111111111';
const view: ManagementView = {
  page: {
    pageId,
    sharing: 'link',
    history: params.get('history') === 'current' ? 'current' : 'shared',
    archived: false,
    epoch: '1',
    retentionDays: 30,
    deleted: false,
    lastUpdateAtMs: null,
    expiresAtMs: null,
    warnings: [],
  },
  root: 'A'.repeat(43),
  revision: '1',
  ownerMember: 'owner',
  members: shareCopy ? [{ id: 'reviewer', role: 'viewer', pages: [pageId] }] : [],
  links: shareCopy
    ? [{ id: '22222222-2222-4222-8222-222222222222', role: 'viewer', pages: [pageId] }]
    : [],
};
const port: ManagementPort = {
  read: async () => view,
  prepare: async (_, selection) => {
    const link =
      selection.operation === 'link.add'
        ? selection.value
        : selection.operation === 'link.remove'
          ? selection.value.replacement
          : null;
    return {
      body: 'fixture-only',
      id: 'operation',
      page: pageId,
      context: view.page,
      operation: selection.operation,
      expiresAt: Date.now() + 60000,
      expectedRevision: '1',
      // New and replacement links carry the captured role and a one-time secret.
      artifact: link
        ? {
            linkId: link.linkId,
            seed: 'A'.repeat(43),
            space: 'a'.repeat(32),
            root: 'A'.repeat(43),
            role:
              artifactRole === 'viewer' || artifactRole === 'commenter' || artifactRole === 'editor'
                ? artifactRole
                : link.role,
          }
        : undefined,
    };
  },
  send: async () => ({
    operationId: 'operation',
    membershipHead: { revision: '2', statementHash: 'A'.repeat(43) },
  }),
  verify: async () => {
    if (unconfirmed && verifications++ === 0) throw new Error('The signed log is not readable yet');
    return view;
  },
  manageProject: async () => {},
  setPinned: async () => {},
  setProjects: async () => {},
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
