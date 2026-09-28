import { render, screen } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { AreaRoster } from './office-directory.js';
import type { AreaPopulation } from './office-population.js';

const empty: AreaPopulation = {
  area: { id: 'area', name: 'Meeting area', binding: { type: 'meeting', roomId: 'room' } },
  members: [],
};

it('does not misrepresent an unavailable room as an empty roster', () => {
  render(<AreaRoster population={empty} select={vi.fn()} />);
  expect(screen.getByRole('status').textContent).toContain('unavailable');
  expect(screen.queryByRole('heading', { name: 'Members · 0' })).toBeNull();
  expect(screen.queryByText('No members yet.')).toBeNull();
  expect(screen.queryByRole('searchbox')).toBeNull();
});

it('distinguishes an existing empty room from unavailable membership', () => {
  render(
    <AreaRoster
      population={{
        ...empty,
        room: { id: 'room', name: 'Design', revision: 1, retired: false, memberIds: [] },
      }}
      select={vi.fn()}
    />
  );
  expect(screen.getByRole('heading', { name: 'Members · 0' })).toBeDefined();
  expect(screen.getByText('No members yet.')).toBeDefined();
  expect(screen.queryByRole('status')).toBeNull();
});
