import { expect, it, vi } from 'vitest';
import { createSnapshotSendState } from './snapshot-send-state.js';

it('composes a replyable question with the exact immutable snapshot reference and reader commands', () => {
  const id = '11111111-1111-4111-8111-111111111111';
  const send = vi.fn();
  const state = createSnapshotSendState({ send }, id);
  state.change('  What is missing?\n', [
    { id: '22222222-2222-4222-8222-222222222222', name: 'Alice' },
  ]);
  state.review();
  expect(state.kind).toBe('request');
  expect(state.getSnapshot().review?.kind).toBeUndefined();
  expect(state.getSnapshot().review?.message).toBe(
    `  What is missing?\n\n\nWhiteboard snapshot: tmt:whiteboard:snapshot:${id}\nRead: tmt office whiteboard snapshot show tmt:whiteboard:snapshot:${id} --json\nImage: tmt office whiteboard snapshot export tmt:whiteboard:snapshot:${id} --output <new-file.png>\nIf you cannot view images, review the structured content and say the image was not inspected.`
  );
  expect(send).not.toHaveBeenCalled();
  state.dispose();
});
