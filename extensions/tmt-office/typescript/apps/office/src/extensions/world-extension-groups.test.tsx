import { mapGeometry } from '../world-map/map-source.js';
import { fireEvent, render, screen, within } from '@testing-library/react';
import { expect, it, vi } from 'vite-plus/test';
import {
  officeWorldFixture,
  WORLD_LOBBY_ID,
} from '../../../../../../../typescript/test/support/office-world.js';
import { defaultCatalog } from '../blocks/block-contract.js';
import type { WorldDocument, WorldObject } from '../world-map/world-contract.js';
import { WHITEBOARD_EXTENSION } from './bundled-extensions.js';
import { bindWorldExtension } from './extension-binding.js';
import { worldExtensionGroups } from './world-extension-groups.js';
import { WorldObjectActions } from './world-object-actions.js';

const meetingId = '20000000-0000-4000-8000-000000000002';
function fixture() {
  const objects: WorldObject[] = [
    [2, 2],
    [18, 2],
    [34, 2],
    [2, 16],
    [18, 16],
  ].map(([x, y], i) => ({
    id: `30000000-0000-4000-8000-00000000000${i + 1}`,
    kind: 'decoration',
    placement: { ...WHITEBOARD_EXTENSION.appearance, x: x!, y: y!, rotation: 0 },
    surface:
      i < 3
        ? { type: 'floor' }
        : {
            type: 'wall',
            axis: 'horizontal',
            face: i === 3 ? 'negative' : 'positive',
            elevation: 3,
          },
    extension: {
      definition: WHITEBOARD_EXTENSION.id,
      binding: { kind: 'whiteboard', documentId: 'lobby' },
    },
  }));
  const base = officeWorldFixture().layout;
  const world: WorldDocument = {
    ...base,
    objects,
    map: {
      ...base.map,
      areas: [
        ...mapGeometry(base.map).areas,
        { id: meetingId, name: 'Design', binding: { type: 'meeting', roomId: meetingId } },
      ],
      floor: Array.from({ length: 16 }, (_, y) => [
        { y, start: 0, end: 16, areaId: WORLD_LOBBY_ID },
        { y, start: 16, end: 32, areaId: meetingId },
        { y, start: 32, end: 48, areaId: null },
      ]).flat(),
    },
  };
  const open = vi.fn();
  const entries = objects.map((object) =>
    bindWorldExtension(object, [WHITEBOARD_EXTENSION], defaultCatalog(), {
      'whiteboard.open': open,
    })!
  );
  return { world, entries, open };
}

it('groups placements by floor and mounted interior face, not by the shared resource ID', () => {
  const { world, entries, open } = fixture();
  const before = structuredClone(world);
  const groups = worldExtensionGroups(world, entries);
  expect(
    groups.map((group) => [group.label, group.entries.map((entry) => entry.instance.id)])
  ).toEqual([
    ['Lobby', [world.objects[0]!.id, world.objects[3]!.id]],
    ['Design', [world.objects[1]!.id]],
    ['Common floor', [world.objects[2]!.id]],
    ['Outside current floor', [world.objects[4]!.id]],
  ]);
  // Removing a designation changes discovery only; it cannot retarget a resource.
  const detached = {
    ...world,
    map: {
      ...mapGeometry(world.map),
      areas: mapGeometry(world.map).areas.filter((area) => area.id !== meetingId),
      floor: mapGeometry(world.map).floor.map((span) =>
        span.areaId === meetingId ? { ...span, areaId: null } : span
      ),
    },
  };
  expect(
    worldExtensionGroups(detached, entries).find((group) => group.areaId === null)!.entries
  ).toEqual([entries[1], entries[2]]);
  expect(world).toEqual(before);
  expect(open).not.toHaveBeenCalled();
});

it('prioritizes current-area tools, keeps common floor accessible and explicitly expands other areas', () => {
  const { world, entries } = fixture();
  const activate = vi.fn(),
    focus = vi.fn();
  render(
    <WorldObjectActions
      groups={worldExtensionGroups(world, entries)}
      areaId={meetingId}
      activate={activate}
      focus={focus}
    />
  );
  const design = screen.getByRole('region', { name: 'Tools in Design' });
  const localButton = within(design).getByRole('button', { name: 'Open whiteboard' });
  fireEvent.click(localButton);
  expect(activate).toHaveBeenCalledExactlyOnceWith(world.objects[1]!.id);
  expect(localButton.getAttribute('aria-describedby')).toBeTruthy();
  expect(within(design).getByText('Tile 18, 2')).toBeTruthy();
  expect(screen.getByRole('region', { name: 'Tools in Common floor' })).toBeTruthy();
  const other = screen.getByText('Other areas · 2').parentElement as HTMLDetailsElement;
  expect(other.open).toBe(false);
  fireEvent.click(screen.getByText('Other areas · 2'));
  expect(other.open).toBe(true);
  expect(
    within(screen.getByRole('region', { name: 'Tools in Lobby' })).getAllByRole('button')
  ).toHaveLength(2);
});
