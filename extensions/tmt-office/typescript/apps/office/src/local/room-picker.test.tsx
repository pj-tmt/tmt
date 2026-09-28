import { act, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import { RoomPicker } from './room-picker.js';
import type { MeetingRoom, RoomPort } from './room-contract.js';
import { DISPATCH_RECIPIENT_LIMIT } from './dispatch-contract.js';

const id = '11111111-1111-4111-8111-111111111111';
const alice = {
  id: '22222222-2222-4222-8222-222222222222',
  name: 'Alice',
  presence: 'offline' as const,
};
const initial = { id, name: 'Design', revision: 1, retired: false, memberIds: [alice.id] };

it('adds an offline agent through the existing room draft without losing members or writing on browse/discard', async () => {
  const user = userEvent.setup();
  const bob = { ...alice, id: '33333333-3333-4333-8333-333333333333', name: 'Bob' };
  const saved = { ...initial, revision: 2, memberIds: [alice.id, bob.id] };
  const port: RoomPort = {
    list: vi.fn(async () => [initial]),
    save: vi.fn().mockRejectedValueOnce(new Error('Response lost')).mockResolvedValueOnce(saved),
    retire: vi.fn(),
  };
  const onSaved = vi.fn();
  render(
    <RoomPicker
      purpose="manage"
      port={port}
      choices={[alice, bob]}
      addMember={bob}
      onSaved={onSaved}
    />
  );
  await screen.findByRole('option', { name: 'Design' });
  await user.selectOptions(screen.getByRole('combobox'), id);
  expect(port.save).not.toHaveBeenCalled();
  await user.click(screen.getByRole('button', { name: 'Add Bob' }));
  expect(screen.getByRole('checkbox', { name: /Alice/ })).toHaveProperty('checked', true);
  expect(screen.getByRole('checkbox', { name: /Bob/ })).toHaveProperty('checked', true);
  await user.click(screen.getByRole('button', { name: 'Discard room draft' }));
  expect(port.save).not.toHaveBeenCalled();
  await user.click(screen.getByRole('button', { name: 'Add Bob' }));
  await user.click(screen.getByRole('button', { name: 'Save room' }));
  expect((await screen.findByRole('alert')).textContent).toContain('Your draft is kept');
  await user.click(screen.getByRole('button', { name: 'Save room' }));
  expect(vi.mocked(port.save).mock.calls.map((call) => call.slice(0, 2))).toEqual([
    [id, { expectedRevision: 1, name: 'Design', memberIds: [alice.id, bob.id] }],
    [id, { expectedRevision: 1, name: 'Design', memberIds: [alice.id, bob.id] }],
  ]);
  expect(onSaved).toHaveBeenCalledExactlyOnceWith(saved);
  expect(initial.memberIds).toEqual([alice.id]);
  expect(port.retire).not.toHaveBeenCalled();
});

it.each(['already joined', 'full'] as const)(
  'does not create an invalid add-member draft when %s',
  async (state) => {
    const user = userEvent.setup();
    const room =
      state === 'already joined'
        ? initial
        : {
            ...initial,
            memberIds: Array.from(
              { length: DISPATCH_RECIPIENT_LIMIT },
              (_, index) => `40000000-0000-4000-8000-${String(index + 1).padStart(12, '0')}`
            ),
          };
    const port: RoomPort = { list: vi.fn(async () => [room]), save: vi.fn(), retire: vi.fn() };
    render(<RoomPicker purpose="manage" port={port} choices={[alice]} addMember={alice} />);
    await screen.findByRole('option', { name: 'Design' });
    await user.selectOptions(screen.getByRole('combobox'), id);
    expect(
      screen.getByRole('button', {
        name: state === 'already joined' ? 'Alice is already a member' : 'Add Alice',
      })
    ).toHaveProperty('disabled', true);
    expect(screen.queryByRole('form', { name: 'Meeting room editor' })).toBeNull();
    expect(port.save).not.toHaveBeenCalled();
  }
);

it('seeds only the chosen agent when creating a room from an agent entry point', async () => {
  const user = userEvent.setup();
  const port: RoomPort = { list: vi.fn(async () => []), save: vi.fn(), retire: vi.fn() };
  render(<RoomPicker purpose="manage" port={port} choices={[alice]} addMember={alice} />);
  await user.click(screen.getByRole('button', { name: 'Create room' }));
  expect(screen.getByRole('checkbox', { name: /Alice/ })).toHaveProperty('checked', true);
  expect(screen.getByRole('textbox', { name: 'Room name' })).toHaveProperty('value', '');
  expect(port.save).not.toHaveBeenCalled();
});
function setup() {
  let saved: MeetingRoom | undefined;
  const port: RoomPort = {
    retire: vi.fn(),
    list: vi.fn(async () => (saved ? [initial, saved] : [initial])),
    save: vi.fn(async (id, input) => {
      saved = {
        id,
        name: input.name,
        revision: input.expectedRevision + 1,
        retired: false,
        memberIds: [...input.memberIds].sort(),
      };
      return saved;
    }),
  };
  const onUse = vi.fn();
  const onDraftChange = vi.fn();
  render(<RoomPicker port={port} choices={[alice]} onUse={onUse} onDraftChange={onDraftChange} />);
  return { port, onUse, onDraftChange };
}

it('requires retirement confirmation and retries the frozen room revision without losing retained resources', async () => {
  const user = userEvent.setup();
  const retired = { ...initial, revision: 2, retired: true };
  const port: RoomPort = {
    list: vi.fn(async () => [initial]),
    save: vi.fn(),
    retire: vi
      .fn()
      .mockRejectedValueOnce(new Error('Response lost'))
      .mockResolvedValueOnce(retired),
  };
  const onSaved = vi.fn();
  render(<RoomPicker purpose="manage" port={port} choices={[alice]} onSaved={onSaved} />);
  await screen.findByRole('option', { name: 'Design' });
  await user.selectOptions(screen.getByRole('combobox'), id);
  await user.click(screen.getByRole('button', { name: 'Retire room' }));
  expect(port.retire).not.toHaveBeenCalled();
  expect(screen.getByText(/Keep the area, furniture/)).toBeTruthy();
  await user.click(screen.getByRole('button', { name: 'Cancel retirement' }));
  expect(port.retire).not.toHaveBeenCalled();
  await user.click(screen.getByRole('button', { name: 'Retire room' }));
  await user.click(screen.getByRole('button', { name: 'Confirm retirement' }));
  await screen.findByText(/Retirement was not confirmed/);
  vi.mocked(port.list).mockResolvedValue([]);
  await user.click(screen.getByRole('button', { name: 'Confirm retirement' }));
  await screen.findByText('No meeting rooms yet. Create one and choose its members.');
  expect(port.retire).toHaveBeenCalledTimes(2);
  for (const call of vi.mocked(port.retire).mock.calls) expect(call.slice(0, 2)).toEqual([id, 1]);
  expect(onSaved).toHaveBeenCalledExactlyOnceWith(retired);
  expect(port.save).not.toHaveBeenCalled();
});

it('requires explicit roster adoption and never expands a selection after refresh', async () => {
  const user = userEvent.setup();
  const { port, onUse } = setup();
  await screen.findByRole('option', { name: 'Design' });
  await user.selectOptions(screen.getByRole('combobox', { name: 'Meeting room' }), id);
  expect(onUse).toHaveBeenLastCalledWith();
  await user.click(screen.getByRole('button', { name: 'Use this roster' }));
  expect(onUse).toHaveBeenLastCalledWith(initial);
  vi.mocked(port.list).mockResolvedValue([
    { ...initial, revision: 2, retired: false, memberIds: [] },
  ]);
  onUse.mockClear();
  await user.click(screen.getByRole('button', { name: 'Refresh rooms' }));
  await screen.findByText('An empty room can be linked to an area but cannot receive requests.');
  expect(
    (screen.getByRole('button', { name: 'Use this roster' }) as HTMLButtonElement).disabled
  ).toBe(true);
  expect(onUse).not.toHaveBeenCalled();
});

it('refreshes after saving and ignores an older list response even if it ignores cancellation', async () => {
  const user = userEvent.setup();
  const { port, onUse } = setup();
  await screen.findByRole('option', { name: 'Design' });
  let resolve!: (rooms: MeetingRoom[]) => void;
  vi.mocked(port.list).mockImplementationOnce(
    () =>
      new Promise((done) => {
        resolve = done;
      })
  );
  await user.click(screen.getByRole('button', { name: 'Refresh rooms' }));
  const oldSignal = vi.mocked(port.list).mock.calls[1]![0]!;
  await user.click(screen.getByRole('button', { name: 'Create room' }));
  await user.type(screen.getByRole('textbox', { name: 'Room name' }), 'New room');
  await user.click(screen.getByRole('checkbox', { name: 'Alice · offline' }));
  await user.click(screen.getByRole('button', { name: 'Save room' }));
  await screen.findByRole('option', { name: 'New room' });
  expect(oldSignal.aborted).toBe(true);
  await act(async () => {
    resolve([initial]);
  });
  expect(screen.getByRole('option', { name: 'New room' })).toBeTruthy();
  await user.click(screen.getByRole('button', { name: 'Use this roster' }));
  expect(onUse).toHaveBeenLastCalledWith(
    expect.objectContaining({ name: 'New room', memberIds: [alice.id] })
  );
});

it('writes one revision-scoped room without selecting it for a request automatically', async () => {
  const user = userEvent.setup();
  const { port, onUse, onDraftChange } = setup();
  await user.click(screen.getByRole('button', { name: 'Create room' }));
  expect(onDraftChange).toHaveBeenLastCalledWith(true);
  const form = within(screen.getByRole('form', { name: 'Meeting room editor' }));
  await user.type(form.getByRole('textbox', { name: 'Room name' }), 'Review');
  await user.click(form.getByRole('checkbox', { name: 'Alice · offline' }));
  expect(port.save).not.toHaveBeenCalled();
  await user.click(form.getByRole('button', { name: 'Save room' }));
  await screen.findByRole('option', { name: 'Review' });
  expect(port.save).toHaveBeenCalledWith(
    expect.any(String),
    { expectedRevision: 0, name: 'Review', memberIds: [alice.id] },
    expect.any(AbortSignal)
  );
  expect(onUse).toHaveBeenLastCalledWith();
  expect(onDraftChange).toHaveBeenLastCalledWith(false);
});

it('keeps the draft and creation UUID after an uncertain save', async () => {
  const user = userEvent.setup();
  const { port } = setup();
  vi.mocked(port.save).mockRejectedValueOnce(new Error('response lost'));
  await user.click(screen.getByRole('button', { name: 'Create room' }));
  await user.type(screen.getByRole('textbox', { name: 'Room name' }), 'Retry');
  await user.click(screen.getByRole('button', { name: 'Save room' }));
  expect((await screen.findByRole('alert')).textContent).toContain('Your draft is kept');
  expect((screen.getByRole('textbox', { name: 'Room name' }) as HTMLInputElement).value).toBe(
    'Retry'
  );
  await user.click(screen.getByRole('button', { name: 'Save room' }));
  expect(vi.mocked(port.save).mock.calls[1]!.slice(0, 2)).toEqual(
    vi.mocked(port.save).mock.calls[0]!.slice(0, 2)
  );
});

it('manages empty rooms independently from audience adoption and reports the saved resource', async () => {
  const user = userEvent.setup();
  const saved = { ...initial, name: 'Empty meeting', memberIds: [] };
  const port: RoomPort = {
    list: vi.fn(async () => [saved]),
    save: vi.fn(async () => saved),
    retire: vi.fn(),
  };
  const onSaved = vi.fn();
  render(<RoomPicker purpose="manage" port={port} choices={[alice]} onSaved={onSaved} />);
  await user.click(screen.getByRole('button', { name: 'Create room' }));
  await user.type(screen.getByRole('textbox', { name: 'Room name' }), saved.name);
  await user.click(screen.getByRole('button', { name: 'Save room' }));
  await screen.findByText('Revision 1 · 0 members');
  expect(onSaved).toHaveBeenCalledExactlyOnceWith(saved);
  expect(port.save).toHaveBeenCalledWith(
    expect.any(String),
    {
      expectedRevision: 0,
      name: saved.name,
      memberIds: [],
    },
    expect.any(AbortSignal)
  );
  expect(screen.queryByRole('button', { name: 'Use this roster' })).toBeNull();
});

it('locks a room target, includes offline members, and never substitutes another roster', async () => {
  const user = userEvent.setup();
  const other = { ...initial, id: '33333333-3333-4333-8333-333333333333', name: 'Other' };
  const port: RoomPort = {
    list: vi.fn(async () => [initial, other]),
    save: vi.fn(),
    retire: vi.fn(),
  };
  const onUse = vi.fn();
  render(<RoomPicker roomId={id} port={port} choices={[alice]} onUse={onUse} />);
  await screen.findByText('Revision 1 · 1 members');
  expect(onUse).not.toHaveBeenCalled();
  expect((screen.getByRole('combobox') as HTMLSelectElement).disabled).toBe(true);
  expect(screen.queryByRole('button', { name: 'Create room' })).toBeNull();
  await user.click(screen.getByRole('button', { name: 'Use this roster' }));
  expect(onUse).toHaveBeenCalledExactlyOnceWith(initial);
  onUse.mockClear();
  vi.mocked(port.list).mockResolvedValue([other]);
  await user.click(screen.getByRole('button', { name: 'Refresh rooms' }));
  expect((await screen.findByRole('alert')).textContent).toContain('No other audience');
  expect(screen.queryByRole('button', { name: 'Use this roster' })).toBeNull();
  expect(onUse).not.toHaveBeenCalled();
});

it('edits a targeted room membership without unlocking the room or adopting a dispatch audience', async () => {
  const user = userEvent.setup();
  const updated = { ...initial, revision: 2, memberIds: [] };
  const port: RoomPort = {
    list: vi.fn(async () => [initial]),
    save: vi.fn(async () => updated),
    retire: vi.fn(),
  };
  const onSaved = vi.fn();
  render(
    <RoomPicker purpose="manage" roomId={id} port={port} choices={[alice]} onSaved={onSaved} />
  );
  await screen.findByText('Revision 1 · 1 members');
  await user.click(screen.getByRole('button', { name: 'Edit room' }));
  await user.click(screen.getByRole('checkbox', { name: 'Alice · offline' }));
  expect(port.save).not.toHaveBeenCalled();
  await user.click(screen.getByRole('button', { name: 'Save room' }));
  expect(port.save).toHaveBeenCalledExactlyOnceWith(
    id,
    { expectedRevision: 1, name: 'Design', memberIds: [] },
    expect.any(AbortSignal)
  );
  expect(onSaved).toHaveBeenCalledExactlyOnceWith(updated);
  expect(screen.queryByRole('button', { name: 'Use this roster' })).toBeNull();
  expect((screen.getByRole('combobox') as HTMLSelectElement).disabled).toBe(true);
});
