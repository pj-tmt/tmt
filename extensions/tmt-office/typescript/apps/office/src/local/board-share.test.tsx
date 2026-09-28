import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import { StrictMode } from 'react';
import { BoardReferenceActions, BoardShare } from './board-share.js';
import type { BoardEntry } from './board-contract.js';
import type { DispatchInput, DispatchReceipt } from './dispatch-contract.js';
import { PROFILE_CATALOG } from '../profiles/profile-contract.js';
import type { ProfilePort } from '../profiles/profile-contract.js';

const thread: BoardEntry = {
  id: '11111111-1111-4111-8111-111111111111',
  threadId: '11111111-1111-4111-8111-111111111111',
  category: { kind: 'general' },
  author: { kind: 'owner' },
  revision: 1,
  deleted: false,
  createdAtMs: 1,
  updatedAtMs: 1,
  title: 'Architecture review',
  body: 'Use the shared request owner.',
};
const aliceId = '22222222-2222-4222-8222-222222222222';
function setup() {
  const profiles: ProfilePort = {
    list: vi.fn(async () => [
      {
        identityId: aliceId,
        identityName: 'Alice',
        exists: false,
        revision: 0,
        updatedAtMs: null,
        presence: 'offline' as const,
        selfReportedStatus: null,
        lifetime: 'saved' as const,
        catalog: PROFILE_CATALOG,
        profile: {
          displayLabel: '',
          description: '',
          appearance: {
            hairStyle: 'short' as const,
            hairColor: 'ink' as const,
            skinTone: 'medium' as const,
            shirtColor: 'blue' as const,
            shirtMark: '',
          },
        },
      },
    ]),
    show: vi.fn(),
    apply: vi.fn(),
  };
  const send = vi.fn(async (input: DispatchInput): Promise<DispatchReceipt> => ({
    operationId: input.operationId,
    createdAtMs: 1,
    items: [{ recipientId: aliceId, requestId: 'req_example', acceptance: 'queued' }],
  }));
  const close = vi.fn();
  const runtime = {
    profiles,
    dispatch: { send },
    requests: { list: vi.fn(), show: vi.fn(), receipt: vi.fn() },
    rooms: { list: vi.fn(async () => []), save: vi.fn(), retire: vi.fn() },
  };
  return { runtime, send, close };
}

it('copies the native thread UUID without asking agents and exposes a manual fallback', async () => {
  const user = userEvent.setup();
  const write = vi.spyOn(navigator.clipboard, 'writeText');
  const ask = vi.fn();
  render(<BoardReferenceActions thread={thread} ask={ask} />);
  await user.click(screen.getByRole('button', { name: 'Copy reference' }));
  expect(write).toHaveBeenCalledWith(thread.id);
  expect(ask).not.toHaveBeenCalled();
  write.mockRejectedValueOnce(new Error('Clipboard unavailable'));
  await user.click(screen.getByRole('button', { name: 'Copy reference' }));
  expect(
    await screen.findByText('Clipboard unavailable. Copy the selected reference manually.')
  ).toBeTruthy();
  const field = screen.getByRole('textbox', { name: 'Local thread reference' }) as HTMLInputElement;
  expect(field.closest('details')!.open).toBe(true);
  expect(document.activeElement).toBe(field);
  expect([field.selectionStart, field.selectionEnd]).toEqual([0, thread.id.length]);
  expect(field.value).toBe(thread.id);
});

it('retains an uncertain request and retries the same live reference without copying board content', async () => {
  const user = userEvent.setup();
  const props = setup();
  props.send.mockRejectedValueOnce(new Error('Lost response'));
  render(
    <StrictMode>
      <BoardShare thread={thread} runtime={props.runtime} close={props.close} />
    </StrictMode>
  );
  await user.click(await screen.findByRole('checkbox', { name: 'Alice · offline' }));
  await user.type(screen.getByRole('textbox', { name: 'Question' }), 'Review this boundary');
  expect(props.send).not.toHaveBeenCalled();
  await user.click(screen.getByRole('button', { name: 'Back to discussion' }));
  expect(props.close).not.toHaveBeenCalled();
  await user.click(screen.getByRole('button', { name: 'Keep composing' }));
  await user.click(screen.getByRole('button', { name: 'Review request' }));
  const expected = `Review this boundary\n\nDiscussion thread: ${thread.id}\nRead: tmt office board show ${thread.id} --json\nThis is a live discussion reference, not a frozen snapshot. Read the current thread before replying.`;
  expect(
    (screen.getByRole('textbox', { name: 'Exact request' }) as HTMLTextAreaElement).value
  ).toBe(expected);
  await user.click(screen.getByRole('button', { name: 'Send request' }));
  await user.click(await screen.findByRole('button', { name: 'Retry send' }));
  await screen.findByText(/Request recorded/);
  expect(props.send).toHaveBeenCalledTimes(2);
  expect(props.send.mock.calls[0]![0]).toEqual(props.send.mock.calls[1]![0]);
  expect(props.send.mock.calls[0]![0]).toMatchObject({
    recipientIds: [aliceId],
    message: expected,
  });
  await user.click(screen.getByRole('button', { name: 'Back to discussion' }));
  await waitFor(() => expect(props.close).toHaveBeenCalledOnce());
});

it('requires explicit discard before leaving an unsent request', async () => {
  const user = userEvent.setup();
  const props = setup();
  render(<BoardShare thread={thread} runtime={props.runtime} close={props.close} />);
  await user.type(screen.getByRole('textbox', { name: 'Question' }), 'Draft');
  await user.click(screen.getByRole('button', { name: 'Back to discussion' }));
  expect(props.close).not.toHaveBeenCalled();
  await user.click(screen.getByRole('button', { name: 'Discard and return' }));
  expect(props.close).toHaveBeenCalledOnce();
  expect(props.send).not.toHaveBeenCalled();
});
