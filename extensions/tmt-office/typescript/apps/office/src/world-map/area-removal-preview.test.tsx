import { fireEvent, render, screen, within } from '@testing-library/react';
import { expect, it } from 'vite-plus/test';
import { officeWorldFixture } from '../../../../../../../typescript/test/support/office-world.js';
import { officePopulation } from '../local/office-population.js';
import { AreaRemovalPreview } from './area-removal-preview.js';
import vectors from '../../../../../contracts/modules-v2-vectors.json';
import { decodeModuleMap } from './module-contract.js';
import { previewModuleRemoval } from './module-authoring.js';

it('explains blocked modular removal without promising that unsupported furniture stays on floor', () => {
  const base = officeWorldFixture().layout;
  const map = decodeModuleMap({ ...vectors.starter, version: 3 });
  const area = map.modules[1]!.area;
  const object = { ...base.objects[0]!, placement: { ...base.objects[0]!.placement, y: -30 } };
  const world = { ...base, map, objects: [object] };
  const population = officePopulation(world.map, [], []).areas.get(area.id)!;
  render(
    <AreaRemovalPreview
      world={world}
      population={population}
      removal={previewModuleRemoval(world, area.id)}
    />
  );
  expect(screen.getByText('Remove Office 01 module')).toBeTruthy();
  expect(screen.getByText('1 placements require attention')).toBeTruthy();
  expect(screen.getByText('Move or explicitly remove the affected placements first.')).toBeTruthy();
  expect(screen.getByText('Occupants return to the primary Lobby.')).toBeTruthy();
  expect(screen.queryByText(/Floor becomes common space/)).toBeNull();
  expect(world.objects).toEqual([object]);
});

it('names the affected space, preserved objects and unavailable members without changing content', () => {
  const world = officeWorldFixture().layout;
  const area = world.map.areas[0]!;
  const member = '20000000-0000-4000-8000-000000000001';
  const population = {
    ...officePopulation(world.map, [], []).areas.get(area.id)!,
    members: [{ identityId: member }],
  };
  const before = structuredClone(world);
  render(<AreaRemovalPreview world={world} population={population} />);
  const preview = screen.getByRole('region', { name: 'Area removal preview' });
  expect(within(preview).getByText('Remove Lobby designation')).toBeTruthy();
  expect(within(preview).getByText('Occupants move to the replacement Lobby.')).toBeTruthy();
  fireEvent.click(within(preview).getByText('1 occupants affected'));
  expect(within(preview).getByText(member)).toBeTruthy();
  fireEvent.click(within(preview).getByText('1 objects kept in place'));
  expect(within(preview).getByText(`desk · ${world.objects[0]!.id}`)).toBeTruthy();
  expect(world).toEqual(before);
});

it('describes meeting detachment as retained membership rather than moving residents', () => {
  const world = officeWorldFixture().layout;
  const population = {
    area: { ...world.map.areas[0]!, binding: { type: 'meeting' as const, roomId: 'Design' } },
    members: [],
  };
  render(<AreaRemovalPreview world={world} population={population} />);
  expect(
    screen.getByText('Room Design and its membership are kept; only this space is unlinked.')
  ).toBeTruthy();
  expect(screen.getByText('0 room members retained')).toBeTruthy();
  expect(screen.queryByText(/Occupants/)).toBeNull();
});
