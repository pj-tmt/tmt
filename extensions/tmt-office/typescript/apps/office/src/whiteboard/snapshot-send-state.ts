import type { DispatchPort } from '../local/dispatch-contract.js';
import { createDispatchComposerState } from '../local/dispatch-composer-state.js';
import { snapshotReference } from './snapshot-reference.js';

/** Snapshot policy adds a fixed reference; shared composition owns delivery and retries. */
export function createSnapshotSendState(
  port: DispatchPort,
  snapshotId: string,
  operationId?: () => string
) {
  const reference = snapshotReference(snapshotId);
  return createDispatchComposerState(
    port,
    {
      kind: 'request',
      message: (question) =>
        `${question}\n\nWhiteboard snapshot: ${reference}\nRead: tmt office whiteboard snapshot show ${reference} --json\nImage: tmt office whiteboard snapshot export ${reference} --output <new-file.png>\nIf you cannot view images, review the structured content and say the image was not inspected.`,
    },
    operationId
  );
}
export type SnapshotSendState = ReturnType<typeof createSnapshotSendState>;
