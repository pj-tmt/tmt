import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import { officeWorldFixture } from '../../../../../../../typescript/test/support/office-world.js';
import { WorldObjectPicker } from './world-object-picker.js';
import type { WorldDocument } from './world-contract.js';

it('prioritizes the current room without losing common, outside or unavailable placements', async () => {
  const user = userEvent.setup();
  const source = officeWorldFixture().layout;
  if (source.map.version !== 1) throw new Error('This fixture exercises explicit floor ownership.');
  const lobby = source.map.primaryLobbyId;
  const first = source.objects[0]!;
  const world: WorldDocument = {
    ...source,
    map: {
      ...source.map,
      areas: [
        ...source.map.areas,
        {
          id: 'studio',
          name: 'Studio',
          binding: { type: 'personal', identityId: null },
        },
      ],
      floor: [
        ...source.map.floor,
        { y: 4, start: 36, end: 44, areaId: null },
        { y: 4, start: 44, end: 60, areaId: 'studio' },
      ],
    },
    objects: [
      first,
      ...[38, 46, 70].map((x, index) => ({
        ...first,
        id: `placement-${index}`,
        placement: { ...first.placement, x },
      })),
    ],
  };
  const before = structuredClone(world);
  const select = vi.fn();
  const view = render(
    <WorldObjectPicker
      world={world}
      catalog={[]}
      areaId="studio"
      value={first.id}
      select={select}
    />
  );
  const picker = screen.getByRole('combobox', { name: 'Object' });
  expect(
    within(picker)
      .getAllByRole('group')
      .map((group) => group.getAttribute('label'))
  ).toEqual(['Studio · Current room (1)', 'Lobby (1)', 'Common floor (1)', 'Outside layout (1)']);
  expect(within(picker).getAllByRole('option')).toHaveLength(5);
  expect(within(picker).getByRole('option', { name: '1 · Unavailable prop' })).toHaveProperty(
    'value',
    first.id
  );
  await user.selectOptions(picker, 'placement-2');
  expect(select).toHaveBeenCalledExactlyOnceWith('placement-2');
  expect(world).toEqual(before);
  view.rerender(
    <WorldObjectPicker
      world={world}
      catalog={[]}
      areaId={lobby}
      value="placement-2"
      select={select}
    />
  );
  expect(within(picker).getAllByRole('group')[0]).toHaveProperty(
    'label',
    'Lobby · Current room (1)'
  );
  expect(picker).toHaveProperty('value', 'placement-2');
});

it('uses the indoor face for a wall object rather than the tile outside the room', () => {
  const source = officeWorldFixture().layout;
  const original = source.objects[0]!;
  const world: WorldDocument = {
    ...source,
    objects: [
      {
        ...original,
        placement: { ...original.placement, x: 36 },
        surface: { type: 'wall', axis: 'vertical', face: 'negative', elevation: 0 },
      },
    ],
  };
  const object = world.objects[0]!;
  render(
    <WorldObjectPicker
      world={world}
      catalog={[]}
      areaId={world.map.primaryLobbyId}
      value={object.id}
      select={vi.fn()}
    />
  );
  expect(screen.getByRole('group').getAttribute('label')).toBe('Lobby · Current room (1)');
});
