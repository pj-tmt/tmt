import { act, fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { useState } from 'react';
import { MeetingCreationForm } from './meeting-creation-form.js';
import type { MeetingRoom, RoomPort } from '../local/room-contract.js';

const room: MeetingRoom = {
  id: '11111111-1111-4111-8111-111111111111',
  name: 'Design',
  memberIds: [],
  revision: 1,
  retired: false,
};
beforeEach(() => {
  vi.stubGlobal(
    'ResizeObserver',
    class {
      observe() {}
      disconnect() {}
    }
  );
});
afterEach(() => vi.unstubAllGlobals());
function setup(port: RoomPort, rooms: MeetingRoom[] = []) {
  const place = vi.fn<(room: MeetingRoom, areaId: string) => boolean>(() => false),
    roomSaved = vi.fn<(room: MeetingRoom) => void>(),
    close = vi.fn();
  function Harness() {
    const [busy, setBusy] = useState(false);
    return (
      <MeetingCreationForm
        port={port}
        rooms={rooms}
        busy={busy}
        onBusyChange={setBusy}
        roomSaved={roomSaved}
        place={place}
        close={close}
      />
    );
  }
  const mounted = render(<Harness />);
  return { ...mounted, place, roomSaved, close };
}

it('retains one confirmed room and area ID after placement fails; retry never writes another room', async () => {
  const user = userEvent.setup();
  const port: RoomPort = {
    list: vi.fn(),
    retire: vi.fn(),
    save: vi.fn(async (id, input) => ({ ...room, id, name: input.name })),
  };
  const { place, roomSaved } = setup(port);
  const input = screen.getByRole('textbox', { name: 'Room name' });
  expect(document.activeElement).toBe(input);
  await user.type(input, 'Planning');
  expect(port.save).not.toHaveBeenCalled();
  await user.click(screen.getByRole('button', { name: 'Save room' }));
  await screen.findByRole('button', { name: 'Retry placement' });
  const receipt = roomSaved.mock.calls[0]![0];
  expect(receipt.name).toBe('Planning');
  expect(place).toHaveBeenCalledExactlyOnceWith(receipt, expect.any(String));
  await user.click(screen.getByRole('button', { name: 'Retry placement' }));
  expect(place.mock.calls[1]).toEqual(place.mock.calls[0]);
  expect(port.save).toHaveBeenCalledTimes(1);
  expect(port.retire).not.toHaveBeenCalled();
});

it('recovers a lost create response by explicit readback and adoption, not a second create', async () => {
  const user = userEvent.setup();
  let stored: MeetingRoom | undefined;
  const port: RoomPort = {
    retire: vi.fn(),
    list: vi.fn(async () => (stored ? [stored] : [])),
    save: vi.fn(async (id, input) => {
      stored = { ...room, id, name: input.name };
      throw new Error('Response lost after commit');
    }),
  };
  const { place } = setup(port);
  await user.type(screen.getByRole('textbox', { name: 'Room name' }), 'Retained room');
  await user.click(screen.getByRole('button', { name: 'Save room' }));
  await screen.findByRole('alert');
  expect(place).not.toHaveBeenCalled();
  await user.click(screen.getByRole('button', { name: 'Check saved room' }));
  await screen.findByRole('region', { name: 'Saved room review' });
  expect(place).not.toHaveBeenCalled();
  await user.click(screen.getByRole('button', { name: 'Use saved room' }));
  expect(place).toHaveBeenCalledExactlyOnceWith(stored, expect.any(String));
  expect(port.save).toHaveBeenCalledTimes(1);
  expect(port.list).toHaveBeenCalledTimes(1);
});

it('places an existing room without writing membership or creating a room', async () => {
  const user = userEvent.setup();
  const port: RoomPort = { list: vi.fn(), save: vi.fn(), retire: vi.fn() };
  const { place } = setup(port, [room]);
  await user.click(screen.getByText('Place an existing room'));
  await user.selectOptions(screen.getByRole('combobox', { name: 'Existing room' }), room.id);
  await user.click(screen.getByRole('button', { name: 'Place selected room' }));
  expect(place).toHaveBeenCalledExactlyOnceWith(room, expect.any(String));
  expect(port.save).not.toHaveBeenCalled();
  expect(port.list).not.toHaveBeenCalled();
});

it('blocks dismissal while saving and ignores a response after disposal', async () => {
  let resolve!: (value: MeetingRoom) => void;
  const port: RoomPort = {
    list: vi.fn(),
    retire: vi.fn(),
    save: vi.fn(
      () =>
        new Promise<MeetingRoom>((done) => {
          resolve = done;
        })
    ),
  };
  const { close, place, unmount } = setup(port);
  fireEvent.change(screen.getByRole('textbox', { name: 'Room name' }), {
    target: { value: 'Pending' },
  });
  fireEvent.submit(screen.getByRole('form', { name: 'Meeting room editor' }));
  fireEvent.keyDown(screen.getByRole('textbox', { name: 'Room name' }), { key: 'Escape' });
  expect(close).not.toHaveBeenCalled();
  const signal = vi.mocked(port.save).mock.calls[0]![2]!;
  unmount();
  expect(signal.aborted).toBe(true);
  await act(async () => resolve(room));
  expect(place).not.toHaveBeenCalled();
});
