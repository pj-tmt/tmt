import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import {
  BUILTIN_DIGEST,
  BUILTIN_PACK,
  BUILTIN_CATALOG,
  MODULAR_LOUNGE_DIGEST,
  DIRECTIONAL_LOUNGE_DIGEST,
  DIRECTIONAL_WORKSTATION_DIGEST,
} from '../props/prop-contract.js';
import { WorldArtLibrary } from './world-art-library.js';

const catalog = [BUILTIN_CATALOG.find((pack) => pack.digest === DIRECTIONAL_WORKSTATION_DIGEST)!];

it('never offers retired pixel basics, including through search, without mutating retained art', async () => {
  const user = userEvent.setup();
  const choose = vi.fn();
  const retained = { digest: BUILTIN_DIGEST, pack: BUILTIN_PACK };
  const before = structuredClone(retained);
  render(<WorldArtLibrary catalog={[retained, ...catalog]} choose={choose} />);
  expect(screen.queryByText('Legacy pixel basics')).toBeNull();
  expect(screen.queryByRole('button', { name: /^Desk$/ })).toBeNull();
  await user.type(screen.getByRole('searchbox'), BUILTIN_PACK.label);
  expect(screen.getByRole('status').textContent).toContain('No matching objects.');
  expect(screen.queryByText('Legacy pixel basics')).toBeNull();
  expect(choose).not.toHaveBeenCalled();
  expect(retained).toEqual(before);
});

it('offers one directional sofa while keeping the legacy choice when its successor is absent', async () => {
  const user = userEvent.setup();
  const choose = vi.fn();
  const oldPack = BUILTIN_CATALOG.find((pack) => pack.digest === MODULAR_LOUNGE_DIGEST)!;
  const newPack = BUILTIN_CATALOG.find((pack) => pack.digest === DIRECTIONAL_LOUNGE_DIGEST)!;
  const view = render(<WorldArtLibrary catalog={[oldPack, newPack]} choose={choose} />);
  await user.click(screen.getByRole('button', { name: 'Workshop velvet sofa' }));
  expect(choose).toHaveBeenLastCalledWith(newPack, 'lounge-sofa');
  view.rerender(<WorldArtLibrary catalog={[oldPack]} choose={choose} />);
  await user.click(screen.getByRole('button', { name: 'Workshop velvet sofa' }));
  expect(choose).toHaveBeenLastCalledWith(oldPack, 'lounge-sofa');
});

it('searches human names and pack names without changing the draft until a picture is chosen', async () => {
  const user = userEvent.setup();
  const choose = vi.fn();
  render(<WorldArtLibrary catalog={catalog} choose={choose} />);
  const search = screen.getByRole('searchbox', { name: 'Search objects' });
  await user.type(search, '  DESK  ');
  const desk = screen.getByRole('button', { name: 'Workshop writing desk' });
  expect(desk.querySelector('svg')).not.toBeNull();
  expect(screen.queryByRole('button', { name: 'Workshop rolling chair' })).toBeNull();
  expect(choose).not.toHaveBeenCalled();
  await user.click(desk);
  expect(choose).toHaveBeenCalledExactlyOnceWith(catalog[0], 'workstation-desk');
  await user.clear(search);
  await user.type(search, catalog[0]!.pack.label);
  expect(screen.getAllByRole('button')).toHaveLength(catalog[0]!.pack.props.length);
});

it('offers a local search reset with no placement or catalog mutation', async () => {
  const user = userEvent.setup();
  const choose = vi.fn();
  render(<WorldArtLibrary catalog={catalog} choose={choose} />);
  await user.type(screen.getByRole('searchbox'), 'nothing-matches-this');
  expect(screen.queryByRole('button', { name: 'Workshop writing desk' })).toBeNull();
  const empty = screen.getByRole('status');
  expect(empty.textContent).toContain('No matching objects.');
  await user.click(within(empty).getByRole('button', { name: 'Clear search' }));
  expect(screen.getByRole('searchbox')).toHaveProperty('value', '');
  expect(screen.getAllByRole('button')).toHaveLength(catalog[0]!.pack.props.length);
  expect(choose).not.toHaveBeenCalled();
});
