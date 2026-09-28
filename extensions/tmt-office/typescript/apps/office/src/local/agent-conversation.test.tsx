import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { AgentConversation } from './agent-conversation.js';
import type { HistoryDetail, HistoryPage } from './request-history-contract.js';
import type { DispatchInput, DispatchReceipt } from './dispatch-contract.js';

const target = { id: '11111111-1111-4111-8111-111111111111', name: 'Alice' };
const requestId = 'req_22222222-2222-4222-8222-222222222222';
const detail: HistoryDetail = {
  requestId,
  recipientId: target.id,
  roomId: null,
  sender: { kind: 'unknown', identityId: null },
  kind: 'request',
  preparedAtMs: 1000,
  delivery: 'queued',
  recipientAcknowledged: true,
  final: { status: 'not_submitted' },
  prompt: {
    status: 'retained',
    message: '<script>not executable</script>\nQuestion',
    messageBytes: 40,
    expiresAtMs: 10000,
  },
};
function fixture() {
  const requests = {
    list: vi.fn(async (): Promise<HistoryPage> => ({
      items: [{ ...detail, final: { status: 'not_submitted' }, preview: 'Question' }],
      nextBefore: null,
    })),
    show: vi.fn(async (): Promise<HistoryDetail> => detail),
    receipt: vi.fn(async (): Promise<DispatchReceipt | null> => null),
  };
  const dispatch = {
    send: vi.fn(async (input: DispatchInput): Promise<DispatchReceipt> => ({
      operationId: input.operationId,
      createdAtMs: 1000,
      items: [{ recipientId: target.id, requestId, acceptance: 'queued' }],
    })),
  };
  return { requests, dispatch };
}
afterEach(() => {
  sessionStorage.clear();
  vi.restoreAllMocks();
});

it('keeps docked refresh out of the toolbar layout and preserves focused drafts and loaded messages', async () => {
  const runtime = fixture();
  const host = document.createElement('div');
  document.body.append(host);
  const view = render(
    <AgentConversation target={target} runtime={runtime} active refreshHost={host} />
  );
  try {
    await screen.findByText('Acknowledged');
    const input = screen.getByRole('textbox', { name: 'Message' }) as HTMLTextAreaElement;
    fireEvent.change(input, { target: { value: 'Keep this draft' } });
    input.focus();
    const toolbar = view.container.querySelector('.chat-toolbar') as HTMLElement;
    expect(toolbar.hidden).toBe(true);
    let finish!: (page: HistoryPage) => void;
    runtime.requests.list.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        })
    );
    fireEvent.click(within(host).getByRole('button', { name: 'Refresh' }));
    expect(
      (within(host).getByRole('button', { name: 'Refresh' }) as HTMLButtonElement).disabled
    ).toBe(true);
    expect(toolbar.hidden).toBe(true);
    expect(screen.queryByText('Updating…')).toBeNull();
    expect(screen.getByText('Acknowledged')).toBeDefined();
    expect(document.activeElement).toBe(input);
    expect(input.value).toBe('Keep this draft');
    await act(async () =>
      finish({
        items: [{ ...detail, final: { status: 'not_submitted' }, preview: 'Question' }],
        nextBefore: null,
      })
    );
    await waitFor(() =>
      expect(
        (within(host).getByRole('button', { name: 'Refresh' }) as HTMLButtonElement).disabled
      ).toBe(false)
    );
    expect(toolbar.hidden).toBe(true);
    expect(document.activeElement).toBe(input);
    expect(input.value).toBe('Keep this draft');
    runtime.requests.list.mockRejectedValueOnce(new Error('Offline'));
    fireEvent.click(within(host).getByRole('button', { name: 'Refresh' }));
    await screen.findByRole('alert');
    expect(input.value).toBe('Keep this draft');
  } finally {
    view.unmount();
    host.remove();
  }
});

it('sends directly to one member and scopes history to the selected room without a fan-out preview', async () => {
  const runtime = fixture();
  runtime.requests.list.mockResolvedValue({ items: [], nextBefore: null });
  const room = { id: '33333333-3333-4333-8333-333333333333', name: 'Design review' };
  render(<AgentConversation target={{ ...target, room }} runtime={runtime} active />);
  await waitFor(() =>
    expect(runtime.requests.list).toHaveBeenCalledWith(
      expect.objectContaining({ recipientId: target.id, roomId: room.id }),
      expect.any(AbortSignal)
    )
  );
  expect(screen.getByText('Room: Design review. Only Alice receives this message.')).toBeDefined();
  fireEvent.change(screen.getByRole('textbox', { name: 'Message' }), {
    target: { value: 'Private review\n' },
  });
  fireEvent.click(screen.getByRole('button', { name: 'Send' }));
  await waitFor(() =>
    expect(runtime.dispatch.send).toHaveBeenCalledExactlyOnceWith(
      expect.objectContaining({
        message: 'Private review\n',
        recipientIds: [target.id],
        room: { kind: 'direct', roomId: room.id },
      }),
      expect.any(AbortSignal)
    )
  );
  expect(screen.queryByRole('button', { name: 'Review request' })).toBeNull();
});

it('keeps acknowledgment distinct from completion and treats message content as inert exact text', async () => {
  const runtime = fixture();
  render(<AgentConversation target={target} runtime={runtime} active />);
  const selected = screen.getByRole('log', { name: 'Conversation with Alice' });
  await waitFor(() => expect(within(selected).getByText('Acknowledged')).toBeTruthy());
  expect(within(selected).getByText('Waiting for a reply')).toBeTruthy();
  expect(selected.querySelector('pre')?.textContent).toBe(
    detail.prompt.status === 'retained' ? detail.prompt.message : ''
  );
  expect(selected.querySelector('script')).toBeNull();
  runtime.requests.show.mockResolvedValue({
    ...detail,
    final: {
      status: 'retained',
      content: 'Done\nwith details',
      bodyBytes: 17,
      submittedAtMs: 2000,
      expiresAtMs: 10000,
    },
  });
  fireEvent.click(screen.getByRole('button', { name: 'Refresh' }));
  await waitFor(() =>
    expect(selected.querySelector('.conversation-reply')?.textContent).toBe('Done\nwith details')
  );
  expect(selected.querySelector('.conversation-reply')?.textContent).toBe('Done\nwith details');
  expect(runtime.dispatch.send).not.toHaveBeenCalled();
});

it('keeps reply attention until the body is loaded in an expanded reading view', async () => {
  const runtime = fixture();
  const final = {
    status: 'retained' as const,
    content: 'Ready to read',
    bodyBytes: 13,
    submittedAtMs: 2000,
    expiresAtMs: 10000,
  };
  runtime.requests.list.mockResolvedValue({
    items: [{ ...detail, final: { ...final, content: undefined }, preview: 'Question' }],
    nextBefore: null,
  });
  let finish!: (value: HistoryDetail) => void;
  runtime.requests.show.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      })
  );
  const cue = vi.fn();
  const view = render(
    <AgentConversation
      target={target}
      runtime={runtime}
      active
      reading={false}
      minimized
      onCueChange={cue}
    />
  );
  await waitFor(() =>
    expect(cue).toHaveBeenLastCalledWith({ kind: 'reply', text: 'Reply ready', requestId })
  );
  view.rerender(<AgentConversation target={target} runtime={runtime} active onCueChange={cue} />);
  expect(cue).toHaveBeenLastCalledWith({ kind: 'reply', text: 'Reply ready', requestId });
  await act(async () => finish({ ...detail, final }));
  await waitFor(() =>
    expect(cue).toHaveBeenLastCalledWith({ kind: 'idle', text: 'Reply received', requestId })
  );
  expect(screen.getByText('Ready to read')).toBeDefined();
  expect(runtime.dispatch.send).not.toHaveBeenCalled();
});

it('sends from the chat input, keeps Shift+Enter for new lines and preserves inactive drafts', async () => {
  const runtime = fixture();
  const dirty = vi.fn();
  const { rerender } = render(
    <AgentConversation target={target} runtime={runtime} active onDirtyChange={dirty} />
  );
  await screen.findByText('Acknowledged');
  const textbox = screen.getByRole('textbox', { name: 'Message' }) as HTMLTextAreaElement;
  fireEvent.change(textbox, { target: { value: 'A question\nwith another line' } });
  expect(runtime.dispatch.send).not.toHaveBeenCalled();
  expect(dirty).toHaveBeenLastCalledWith(true);
  rerender(
    <AgentConversation target={target} runtime={runtime} active={false} onDirtyChange={dirty} />
  );
  expect(textbox.value).toBe('A question\nwith another line');
  rerender(<AgentConversation target={target} runtime={runtime} active onDirtyChange={dirty} />);
  fireEvent.keyDown(textbox, { key: 'Enter', shiftKey: true });
  expect(runtime.dispatch.send).not.toHaveBeenCalled();
  fireEvent.keyDown(textbox, { key: 'Enter', isComposing: true });
  expect(runtime.dispatch.send).not.toHaveBeenCalled();
  fireEvent.keyDown(textbox, { key: 'Enter' });
  await waitFor(() => expect(textbox.value).toBe(''));
  expect(runtime.dispatch.send).toHaveBeenCalledExactlyOnceWith(
    {
      operationId: expect.any(String),
      recipientIds: [target.id],
      message: 'A question\nwith another line',
    },
    expect.any(AbortSignal)
  );
  expect(dirty).toHaveBeenLastCalledWith(false);
  expect(screen.queryByRole('button', { name: 'Review request' })).toBeNull();
  fireEvent.change(textbox, { target: { value: 'Do not lose this silently.' } });
  expect(screen.queryByRole('button', { name: 'Discard draft' })).toBeNull();
  expect(textbox.value).toBe('Do not lose this silently.');
});

it('retains explicit confirmed recovery discard after an uncertain send', async () => {
  const runtime = fixture();
  runtime.dispatch.send.mockRejectedValue(new Error('Response lost'));
  render(<AgentConversation target={target} runtime={runtime} active />);
  await screen.findByText('Acknowledged');
  const textbox = screen.getByRole('textbox', { name: 'Message' }) as HTMLTextAreaElement;
  fireEvent.change(textbox, { target: { value: 'Do not lose this silently.' } });
  expect(screen.queryByRole('button', { name: 'Discard draft' })).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: 'Send' }));
  await screen.findByRole('button', { name: 'Retry' });
  fireEvent.click(screen.getByRole('button', { name: 'Discard draft' }));
  expect(textbox.value).toBe('Do not lose this silently.');
  fireEvent.click(screen.getByRole('button', { name: 'Confirm discard' }));
  expect(textbox.value).toBe('');
});

it('can release a frozen unsent message when browser journaling fails', async () => {
  const runtime = fixture();
  render(<AgentConversation target={target} runtime={runtime} active />);
  await screen.findByText('Acknowledged');
  vi.spyOn(Object.getPrototypeOf(sessionStorage) as Storage, 'setItem').mockImplementation(() => {
    throw new DOMException('Full', 'QuotaExceededError');
  });
  const textbox = screen.getByRole('textbox', { name: 'Message' }) as HTMLTextAreaElement;
  fireEvent.change(textbox, { target: { value: 'Not sent' } });
  fireEvent.click(screen.getByRole('button', { name: 'Send' }));
  await screen.findByText(/Could not preserve this pending request/);
  expect(runtime.dispatch.send).not.toHaveBeenCalled();
  expect(textbox.readOnly).toBe(true);
  fireEvent.click(screen.getByRole('button', { name: 'Discard draft' }));
  fireEvent.click(screen.getByRole('button', { name: 'Confirm discard' }));
  expect(textbox.readOnly).toBe(false);
  expect(textbox.value).toBe('');
});

it('aborts hidden-tab reads and resumes on visibility without dispatching work', async () => {
  const runtime = fixture();
  let signal: AbortSignal | undefined;
  runtime.requests.list.mockImplementation((_query?: unknown, lifetime?: AbortSignal) => {
    signal = lifetime;
    return new Promise<HistoryPage>(() => {});
  });
  render(<AgentConversation target={target} runtime={runtime} active />);
  await waitFor(() => expect(runtime.requests.list).toHaveBeenCalledTimes(1));
  const hidden = vi.spyOn(document, 'hidden', 'get').mockReturnValue(true);
  act(() => document.dispatchEvent(new Event('visibilitychange')));
  expect(signal?.aborted).toBe(true);
  hidden.mockReturnValue(false);
  act(() => document.dispatchEvent(new Event('visibilitychange')));
  expect(runtime.requests.list).toHaveBeenCalledTimes(2);
  expect(signal?.aborted).toBe(false);
  expect(runtime.dispatch.send).not.toHaveBeenCalled();
});

it('requires explicit discard of an unreadable pending record before composing new work', async () => {
  const runtime = fixture();
  sessionStorage.setItem(`tmt.office.pending-request.${target.id}`, '{invalid');
  render(<AgentConversation target={target} runtime={runtime} active />);
  await screen.findByText(/Pending request recovery is unavailable/);
  expect((screen.getByRole('button', { name: 'Send' }) as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(screen.getByRole('button', { name: 'Discard draft' }));
  fireEvent.click(screen.getByRole('button', { name: 'Keep message' }));
  expect(sessionStorage.length).toBe(1);
  fireEvent.click(screen.getByRole('button', { name: 'Discard draft' }));
  fireEvent.click(screen.getByRole('button', { name: 'Confirm discard' }));
  expect(sessionStorage.length).toBe(0);
  expect((screen.getByRole('textbox', { name: 'Message' }) as HTMLTextAreaElement).disabled).toBe(
    false
  );
  expect(runtime.dispatch.send).not.toHaveBeenCalled();
});
