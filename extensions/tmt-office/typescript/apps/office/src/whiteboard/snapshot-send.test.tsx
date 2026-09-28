import { act, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import { PROFILE_CATALOG } from '../profiles/profile-contract.js';
import type { ProfilePort, ProfileProjection } from '../profiles/profile-contract.js';
import type { DispatchInput, DispatchReceipt } from '../local/dispatch-contract.js';
import { DispatchRejected } from '../local/dispatch-contract.js';
import { WhiteboardSnapshotSend } from './snapshot-send.js';

const snapshotId = '11111111-1111-4111-8111-111111111111';
const aliceId = '22222222-2222-4222-8222-222222222222';
const bobId = '33333333-3333-4333-8333-333333333333';
function profile(identityId: string, identityName: string): ProfileProjection {
  return {
    identityId,
    identityName,
    exists: false,
    revision: 0,
    updatedAtMs: null,
    presence: 'offline' as const,
    selfReportedStatus: null,
    lifetime: 'saved',
    catalog: PROFILE_CATALOG,
    profile: {
      displayLabel: 'Not a routing name',
      description: '',
      appearance: {
        hairStyle: 'short',
        hairColor: 'ink',
        skinTone: 'medium',
        shirtColor: 'blue',
        shirtMark: '',
      },
    },
  };
}
function setup() {
  const profiles: ProfilePort = {
    list: vi.fn(async () => [profile(aliceId, 'Alice'), profile(bobId, 'Bob')]),
    show: vi.fn(),
    apply: vi.fn(),
  };
  const send = vi.fn(async (input: DispatchInput): Promise<DispatchReceipt> => ({
    operationId: input.operationId,
    createdAtMs: 100,
    items: input.recipientIds.map((recipientId) => ({
      recipientId,
      requestId: `req_${crypto.randomUUID()}`,
      acceptance: recipientId === bobId ? 'recipientUnavailable' : 'queued',
    })),
  }));
  const onDraftChange = vi.fn();
  const props = {
    snapshotId,
    profiles,
    rooms: { list: vi.fn(async () => []), save: vi.fn(), retire: vi.fn() },
    dispatch: { send },
    requests: { list: vi.fn(), show: vi.fn(), receipt: vi.fn() },
    onDraftChange,
  };
  return { ...props, send };
}

it('does not offer an uncertain retry after a definite rejection', async () => {
  const user = userEvent.setup();
  const props = setup();
  props.send.mockRejectedValueOnce(new DispatchRejected('operationConflict'));
  render(<WhiteboardSnapshotSend {...props} />);
  await user.click(await screen.findByRole('checkbox', { name: 'Alice · offline' }));
  await user.type(screen.getByRole('textbox', { name: 'Question' }), 'Review');
  await user.click(screen.getByRole('button', { name: 'Review request' }));
  await user.click(screen.getByRole('button', { name: 'Send request' }));
  expect((await screen.findByRole('alert')).textContent).toContain('Do not retry');
  expect(screen.queryByRole('button', { name: 'Retry send' })).toBeNull();
  expect(screen.queryByRole('button', { name: 'Edit request' })).toBeNull();
  await user.click(screen.getByRole('button', { name: 'Discard request' }));
  expect(screen.getByRole('group', { name: 'Discard request confirmation' }).textContent).toContain(
    'does not cancel any queued work'
  );
  expect(props.send).toHaveBeenCalledTimes(1);
});

it('requires explicit audience/message review, retains drafts while hidden, and distinguishes unavailable from queued', async () => {
  const user = userEvent.setup();
  const props = setup();
  const { rerender } = render(
    <div>
      <WhiteboardSnapshotSend {...props} />
    </div>
  );
  await user.click(await screen.findByRole('checkbox', { name: 'Alice · offline' }));
  await user.click(screen.getByRole('checkbox', { name: 'Bob · offline' }));
  await user.type(screen.getByRole('textbox', { name: 'Question' }), 'Review the plan');
  expect(props.send).not.toHaveBeenCalled();
  expect(props.onDraftChange).toHaveBeenLastCalledWith(true);
  rerender(
    <div hidden>
      <WhiteboardSnapshotSend {...props} />
    </div>
  );
  rerender(
    <div>
      <WhiteboardSnapshotSend {...props} />
    </div>
  );
  expect((screen.getByRole('textbox', { name: 'Question' }) as HTMLTextAreaElement).value).toBe(
    'Review the plan'
  );
  await user.click(screen.getByRole('button', { name: 'Review request' }));
  const preview = screen.getByRole('textbox', { name: 'Exact request' }) as HTMLTextAreaElement;
  expect(preview.readOnly).toBe(true);
  expect(preview.value).toContain(`tmt:whiteboard:snapshot:${snapshotId}`);
  expect(
    within(screen.getByRole('list', { name: 'Confirmed recipients' }))
      .getAllByRole('listitem')
      .map((item) => item.textContent)
  ).toEqual(['Alice', 'Bob']);
  expect(props.send).not.toHaveBeenCalled();
  await user.click(screen.getByRole('button', { name: 'Send request' }));
  expect(props.send.mock.calls[0]![0]).toMatchObject({
    message: preview.value,
    recipientIds: [aliceId, bobId],
  });
  expect((await screen.findByRole('status')).textContent).toContain('not read or completed');
  expect(screen.getByText('Unavailable · not queued', { exact: false })).toBeDefined();
  expect(props.onDraftChange).toHaveBeenLastCalledWith(false);
});

it('keeps uncertain sends frozen and retries exactly once instead of replacing an already accepted request', async () => {
  const user = userEvent.setup();
  const props = setup();
  props.send.mockRejectedValueOnce(new Error('lost response'));
  render(<WhiteboardSnapshotSend {...props} />);
  await user.click(await screen.findByRole('checkbox', { name: 'Alice · offline' }));
  await user.type(screen.getByRole('textbox', { name: 'Question' }), 'Review');
  await user.click(screen.getByRole('button', { name: 'Review request' }));
  await user.click(screen.getByRole('button', { name: 'Send request' }));
  expect((await screen.findByRole('alert')).textContent).toContain('may already be queued');
  expect(screen.queryByRole('button', { name: 'Edit request' })).toBeNull();
  await user.click(screen.getByRole('button', { name: 'Discard request' }));
  expect(screen.getByRole('group', { name: 'Discard request confirmation' }).textContent).toContain(
    'does not cancel any queued work'
  );
  await user.click(screen.getByRole('button', { name: 'Keep request' }));
  await user.click(screen.getByRole('button', { name: 'Retry send' }));
  expect((await screen.findByRole('status')).textContent).toContain('Request recorded');
  expect(props.send).toHaveBeenCalledTimes(2);
  expect(props.send.mock.calls[1]![0]).toBe(props.send.mock.calls[0]![0]);
});

it('recovers directory failures without losing the question, warns before unload, and requires explicit discard', async () => {
  const user = userEvent.setup();
  const props = setup();
  vi.mocked(props.profiles.list).mockRejectedValueOnce(new Error('offline'));
  render(<WhiteboardSnapshotSend {...props} />);
  expect((await screen.findByRole('alert')).textContent).toContain('Could not load agents');
  await user.type(screen.getByRole('textbox', { name: 'Question' }), 'Keep this question');
  const unloading = new Event('beforeunload', { cancelable: true });
  window.dispatchEvent(unloading);
  expect(unloading.defaultPrevented).toBe(true);
  await user.click(screen.getByRole('button', { name: 'Retry loading agents' }));
  await screen.findByRole('checkbox', { name: 'Alice · offline' });
  expect((screen.getByRole('textbox', { name: 'Question' }) as HTMLTextAreaElement).value).toBe(
    'Keep this question'
  );
  await user.click(screen.getByRole('button', { name: 'Discard request' }));
  expect(props.onDraftChange).toHaveBeenLastCalledWith(true);
  await user.click(screen.getByRole('button', { name: 'Confirm discard' }));
  expect(props.onDraftChange).toHaveBeenLastCalledWith(false);
  const clean = new Event('beforeunload', { cancelable: true });
  window.dispatchEvent(clean);
  expect(clean.defaultPrevented).toBe(false);
  expect(props.send).not.toHaveBeenCalled();
});

it('does not replace a new directory with a late response from an old profile port', async () => {
  const props = setup();
  let finish!: (profiles: ProfileProjection[]) => void;
  vi.mocked(props.profiles.list).mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      })
  );
  const { rerender } = render(<WhiteboardSnapshotSend {...props} />);
  await waitFor(() => expect(props.profiles.list).toHaveBeenCalledTimes(1));
  rerender(
    <WhiteboardSnapshotSend {...props} profiles={{ ...props.profiles, list: async () => [] }} />
  );
  await screen.findByText('No agents yet. Create an identity with tmt first.');
  await act(async () => {
    finish([profile(aliceId, 'Alice')]);
  });
  expect(screen.queryByRole('checkbox')).toBeNull();
});

it('allows a disappeared selection to be removed after refresh without discarding the question', async () => {
  const user = userEvent.setup();
  const props = setup();
  render(<WhiteboardSnapshotSend {...props} />);
  await user.click(await screen.findByRole('checkbox', { name: 'Alice · offline' }));
  await user.click(screen.getByRole('checkbox', { name: 'Bob · offline' }));
  await user.type(screen.getByRole('textbox', { name: 'Question' }), 'Keep this draft');
  vi.mocked(props.profiles.list).mockResolvedValue([profile(aliceId, 'Alice')]);
  await user.click(screen.getByRole('button', { name: 'Refresh agents' }));
  await user.click(
    await screen.findByRole('checkbox', { name: 'Bob · selected, not in current list' })
  );
  expect((screen.getByRole('textbox', { name: 'Question' }) as HTMLTextAreaElement).value).toBe(
    'Keep this draft'
  );
  await user.click(screen.getByRole('button', { name: 'Review request' }));
  expect(
    within(screen.getByRole('list', { name: 'Confirmed recipients' }))
      .getAllByRole('listitem')
      .map((item) => item.textContent)
  ).toEqual(['Alice']);
  expect(props.send).not.toHaveBeenCalled();
});
