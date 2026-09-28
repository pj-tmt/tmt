import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { useAgentConversation } from './use-agent-conversation.js';
import type { DispatchReceipt } from './dispatch-contract.js';
import type { HistoryPage } from './request-history-contract.js';

const alice = { id: '11111111-1111-4111-8111-111111111111', name: 'Alice' };
const bob = { id: '22222222-2222-4222-8222-222222222222', name: 'Bob' };
function fixture() {
  return {
    requests: {
      list: vi.fn(async (): Promise<HistoryPage> => ({ items: [], nextBefore: null })),
      show: vi.fn(),
      receipt: vi.fn(async (): Promise<DispatchReceipt | null> => null),
    },
    dispatch: { send: vi.fn() },
  };
}
function Harness({
  runtime,
  suspended = false,
  initial = false,
}: {
  runtime: ReturnType<typeof fixture>;
  suspended?: boolean;
  initial?: boolean;
}) {
  const chat = useAgentConversation(runtime, {
    initial: initial ? alice : undefined,
    suspended,
    info: (target) => <p>{target.name} information</p>,
    portrait: () => null,
    closed: () => undefined,
  });
  return (
    <>
      <button onClick={() => chat.open(alice)}>Open Alice</button>
      <button onClick={() => chat.open(bob)}>Open Bob</button>
      {chat.render()}
    </>
  );
}
afterEach(() => sessionStorage.clear());

it('opens initial Info without polling and keeps a draft through tabs, layout selection and closing', async () => {
  const runtime = fixture();
  const view = render(<Harness runtime={runtime} initial />);
  expect(screen.getByText('Alice information')).toBeDefined();
  expect(runtime.requests.list).not.toHaveBeenCalled();
  expect(document.activeElement).toBe(screen.getByRole('heading', { name: 'Alice' }));
  fireEvent.click(screen.getByRole('tab', { name: 'Chat' }));
  const input = await screen.findByRole('textbox', { name: 'Message' });
  fireEvent.change(input, { target: { value: 'Keep this draft' } });
  fireEvent.click(screen.getByRole('tab', { name: 'Info' }));
  expect(screen.queryByRole('button', { name: 'Minimize agent conversation' })).toBeNull();
  view.rerender(<Harness runtime={runtime} suspended />);
  expect(screen.queryByRole('dialog', { name: 'Agent conversation' })).toBeNull();
  view.rerender(<Harness runtime={runtime} />);
  fireEvent.click(screen.getByRole('tab', { name: 'Chat' }));
  expect(screen.getByRole('textbox', { name: 'Message' })).toBe(input);
  expect(input).toHaveProperty('value', 'Keep this draft');
  fireEvent.click(screen.getByRole('button', { name: 'Close agent conversation' }));
  fireEvent.click(screen.getByRole('button', { name: 'Open Alice' }));
  expect(screen.getByRole('textbox', { name: 'Message' })).toBe(input);
  expect(input).toHaveProperty('value', 'Keep this draft');
  expect(runtime.dispatch.send).not.toHaveBeenCalled();
});

it('requires explicit discard before changing a dirty target and keeps the original target on cancel', async () => {
  const runtime = fixture();
  render(<Harness runtime={runtime} />);
  fireEvent.click(screen.getByRole('button', { name: 'Open Alice' }));
  fireEvent.change(await screen.findByRole('textbox', { name: 'Message' }), {
    target: { value: 'For Alice only' },
  });
  fireEvent.click(screen.getByRole('button', { name: 'Open Bob' }));
  expect(screen.getByRole('region', { name: 'Switch conversation confirmation' })).toBeDefined();
  expect(screen.queryByRole('textbox', { name: 'Message' })).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: 'Keep this conversation' }));
  expect(screen.getByRole('textbox', { name: 'Message' })).toHaveProperty(
    'value',
    'For Alice only'
  );
  fireEvent.click(screen.getByRole('button', { name: 'Open Bob' }));
  fireEvent.click(screen.getByRole('button', { name: 'Switch conversation' }));
  expect(await screen.findByRole('textbox', { name: 'Message' })).toHaveProperty('value', '');
  expect(screen.getByRole('heading', { name: 'Bob' })).toBeDefined();
  await waitFor(() =>
    expect(runtime.requests.list).toHaveBeenCalledWith(
      expect.objectContaining({ recipientId: bob.id }),
      expect.any(AbortSignal)
    )
  );
  expect(runtime.dispatch.send).not.toHaveBeenCalled();
});

it('aborts observation when closed or suspended and resumes only when selected again', async () => {
  const runtime = fixture();
  let signal: AbortSignal | undefined;
  runtime.requests.list.mockImplementation((_query?: unknown, lifetime?: AbortSignal) => {
    signal = lifetime;
    return new Promise<HistoryPage>(() => {});
  });
  const view = render(<Harness runtime={runtime} />);
  fireEvent.click(screen.getByRole('button', { name: 'Open Alice' }));
  await waitFor(() => expect(signal).toBeDefined());
  view.rerender(<Harness runtime={runtime} suspended />);
  expect(signal?.aborted).toBe(true);
  view.rerender(<Harness runtime={runtime} />);
  expect(signal?.aborted).toBe(false);
  fireEvent.click(screen.getByRole('button', { name: 'Close agent conversation' }));
  expect(signal?.aborted).toBe(true);
  expect(runtime.dispatch.send).not.toHaveBeenCalled();
});
