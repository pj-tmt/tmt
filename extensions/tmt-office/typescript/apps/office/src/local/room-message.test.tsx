import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { LocalRuntimeContext, startLocalRuntime } from './local-runtime.js';
import { useRoomMessage } from './room-message.js';

const alice = '11111111-1111-4111-8111-111111111111';
const design = {
  id: '22222222-2222-4222-8222-222222222222',
  name: 'Design',
  revision: 1,
  retired: false,
  memberIds: [alice],
};
const planning = { ...design, id: '33333333-3333-4333-8333-333333333333', name: 'Planning' };
afterEach(() => {
  sessionStorage.clear();
  vi.restoreAllMocks();
});
function Host() {
  const message = useRoomMessage();
  return (
    <>
      <button onClick={() => message.open(design)}>Open Design</button>
      <button onClick={() => message.open(planning)}>Open Planning</button>
      {message.panel}
    </>
  );
}
function setup() {
  history.replaceState(null, '', `/local#token=${'a'.repeat(43)}`);
  const runtime = startLocalRuntime(window.location);
  vi.spyOn(runtime.profiles, 'list').mockResolvedValue([]);
  vi.spyOn(runtime.rooms, 'list').mockResolvedValue([design, planning]);
  const send = vi.spyOn(runtime.dispatch, 'send').mockRejectedValue(new Error('Response lost'));
  const lookup = vi.spyOn(runtime.requests, 'receipt').mockResolvedValue(null);
  const view = render(
    <LocalRuntimeContext.Provider value={runtime}>
      <Host />
    </LocalRuntimeContext.Provider>
  );
  return {
    send,
    lookup,
    close: () => {
      view.unmount();
      runtime.dispose();
    },
  };
}
const click = (name: string) => fireEvent.click(screen.getByRole('button', { name }));

it('retains a closed draft and protects switching rooms until explicit discard', async () => {
  const fixture = setup();
  click('Open Design');
  const input = await screen.findByRole('textbox', { name: 'Message' });
  fireEvent.change(input, { target: { value: 'Keep this draft' } });
  click('Close message room');
  click('Open Design');
  expect(screen.getByRole('textbox', { name: 'Message' })).toBe(input);
  click('Close message room');
  click('Open Planning');
  expect(screen.getByRole('region', { name: 'Switch room message confirmation' })).toBeTruthy();
  expect((screen.getByRole('textbox') as HTMLTextAreaElement).value).toBe('Keep this draft');
  click('Discard draft and switch room');
  await screen.findByRole('heading', { name: 'Message Planning' });
  await waitFor(() => expect((screen.getByRole('textbox') as HTMLTextAreaElement).value).toBe(''));
  expect(fixture.send).not.toHaveBeenCalled();
  fixture.close();
});

it('blocks abandoning an uncertain operation and checks its receipt on reopen without resending', async () => {
  const fixture = setup();
  click('Open Design');
  const input = await screen.findByRole('textbox', { name: 'Message' });
  fireEvent.change(input, { target: { value: 'Do this once' } });
  await screen.findByRole('button', { name: 'Use this roster' });
  click('Use this roster');
  click('Review request');
  click('Send request');
  await screen.findByRole('button', { name: 'Retry send' });
  const frozen = fixture.send.mock.calls[0]![0];
  click('Close message room');
  click('Open Planning');
  await waitFor(() => expect(fixture.lookup).toHaveBeenCalled());
  const switchButton = screen.getByRole('button', { name: 'Discard draft and switch room' });
  expect((switchButton as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(switchButton);
  expect(screen.getByRole('heading', { name: 'Message Design' })).toBeTruthy();
  expect(fixture.send).toHaveBeenCalledTimes(1);
  expect(fixture.lookup).toHaveBeenCalledWith(frozen.operationId, expect.any(AbortSignal));
  click('Discard request');
  expect(screen.getByText(/does not cancel any queued work/)).toBeTruthy();
  click('Confirm discard');
  await waitFor(() => expect((switchButton as HTMLButtonElement).disabled).toBe(false));
  click('Discard draft and switch room');
  await screen.findByRole('heading', { name: 'Message Planning' });
  expect(fixture.send).toHaveBeenCalledTimes(1);
  fixture.close();
});
