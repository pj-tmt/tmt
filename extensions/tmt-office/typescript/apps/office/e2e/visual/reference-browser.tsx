/** Retained reference views with local values; no runtime or dispatch capability. */
import { createRoot } from 'react-dom/client';
import '../../src/styles.css';
import '../../src/local/board.css';
import '../../src/whiteboard/snapshot-review.css';
import { BoardReferenceActions } from '../../src/local/board-share.js';
import { SnapshotReferenceView } from '../../src/whiteboard/snapshot-reference-view.js';
import { OfficeDirectory } from '../../src/local/office-directory.js';
import type { BoardEntry } from '../../src/local/board-contract.js';

const id = '11111111-1111-4111-8111-111111111111';
const thread: BoardEntry = {
  id,
  threadId: id,
  category: { kind: 'general' },
  author: { kind: 'owner' },
  revision: 1,
  deleted: false,
  createdAtMs: 1,
  updatedAtMs: 1,
  title: 'Architecture review',
  body: 'Shared request owner.',
};
createRoot(document.getElementById('root')!).render(
  <>
    <OfficeDirectory
      population={{ areas: new Map(), identities: new Map(), homes: new Map() }}
      select={() => {}}
    />
    <h2>Discussion</h2>
    <BoardReferenceActions thread={thread} ask={() => {}} />
    <h2>Whiteboard snapshot</h2>
    <SnapshotReferenceView id={id} />
  </>
);
