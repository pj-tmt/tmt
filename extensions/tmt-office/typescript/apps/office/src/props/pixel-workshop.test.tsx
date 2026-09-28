import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { PixelWorkshop } from './pixel-workshop.js';
import { newPixelDraft, paintPixels, pixelDraftPack } from './pixel-draft.js';
import type { PropCatalogPage, PropInstallInput } from './prop-catalog-contract.js';
import type { CatalogPack } from './prop-contract.js';

const saved: CatalogPack = {
  digest: `sha256:${'a'.repeat(64)}`,
  pack: pixelDraftPack(paintPixels(newPixelDraft(), [{ x: 2, y: 3 }], 2)),
};
function fixture() {
  return {
    list: vi.fn(async (): Promise<PropCatalogPage> => ({
      revision: 0,
      entries: [],
      excluded: [],
      nextCursor: null,
    })),
    load: vi.fn(async () => saved),
    install: vi.fn(async (_input: PropInstallInput) => ({
      revision: 1,
      digest: saved.digest,
      changed: true,
    })),
  };
}

it('supports keyboard paint, palette edits, eraser and undo without writing until explicit Save', async () => {
  const port = fixture();
  const add = vi.fn();
  render(<PixelWorkshop port={port} canAdd add={add} />);
  await screen.findByText('No custom artwork on this page.');
  const canvas = screen.getByRole('group', { name: 'Pixel drawing canvas' });
  const save = screen.getByRole('button', { name: 'Save artwork to library' }) as HTMLButtonElement;
  expect(save.disabled).toBe(true);
  fireEvent.keyDown(canvas, { key: 'ArrowRight' });
  fireEvent.keyDown(canvas, { key: 'ArrowDown' });
  fireEvent.keyDown(canvas, { key: ' ' });
  expect(save.disabled).toBe(false);
  fireEvent.change(screen.getByLabelText('Selected color'), { target: { value: '#123456' } });
  fireEvent.click(screen.getByRole('button', { name: /^Eraser$/ }));
  fireEvent.keyDown(canvas, { key: 'Enter' });
  expect(save.disabled).toBe(true);
  fireEvent.click(screen.getByRole('button', { name: 'Undo pixels' }));
  expect(save.disabled).toBe(false);
  fireEvent.change(screen.getByLabelText('Artwork name'), { target: { value: 'Moon portrait' } });
  expect(port.install).not.toHaveBeenCalled();
  expect(add).not.toHaveBeenCalled();
  fireEvent.click(save);
  await screen.findByText(/Artwork saved/);
  const pack = JSON.parse(port.install.mock.calls[0]![0].document);
  expect(pack).toMatchObject({ label: 'Moon portrait', license: 'LicenseRef-Private' });
  expect(pack.palette[3]).toBe('#123456ff');
  expect(pack.props[0].frames[0][1].slice(2, 4)).toBe('03');
  expect(new Set(pack.props[0].frames.map(JSON.stringify)).size).toBe(1);
  expect(add).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button', { name: 'Add saved artwork to layout draft' }));
  expect(add).toHaveBeenCalledWith({ digest: saved.digest, pack }, 'artwork');
  expect(port.install).toHaveBeenCalledTimes(1);
});

it('freezes all drawing inputs after an uncertain Save, retains the draft and retries once explicitly', async () => {
  const port = fixture();
  port.install.mockRejectedValueOnce(new Error('Lost receipt'));
  render(<PixelWorkshop port={port} canAdd add={vi.fn()} />);
  await screen.findByText('No custom artwork on this page.');
  const canvas = screen.getByRole('group', { name: 'Pixel drawing canvas' });
  fireEvent.keyDown(canvas, { key: ' ' });
  fireEvent.click(screen.getByRole('button', { name: 'Save artwork to library' }));
  await screen.findByText(/Save was not confirmed/);
  expect(screen.getByLabelText('Artwork name').closest('fieldset')!.disabled).toBe(true);
  expect(canvas.getAttribute('aria-disabled')).toBe('true');
  fireEvent.keyDown(canvas, { key: 'Delete' });
  fireEvent.click(screen.getByRole('button', { name: 'Retry exact save' }));
  await screen.findByText(/Artwork saved/);
  expect(port.install.mock.calls[1]![0]).toEqual(port.install.mock.calls[0]![0]);
  expect(
    JSON.parse(port.install.mock.calls[1]![0].document).props[0].frames[0][0].slice(0, 2)
  ).toBe('03');
});

it('discovers unplaced art with paged metadata and loads only a selected pack; failed placement is not success', async () => {
  const port = fixture();
  port.list.mockResolvedValueOnce({
    revision: 0,
    entries: [{ digest: saved.digest, label: 'Stored moon' }],
    excluded: [],
    nextCursor: 'page2',
  });
  const add = vi.fn(() => {
    throw new Error('Paint floor in this area before adding an object.');
  });
  render(<PixelWorkshop port={port} canAdd add={add} />);
  fireEvent.click(screen.getByRole('button', { name: 'Saved library' }));
  fireEvent.click(await screen.findByRole('button', { name: 'Stored moon' }));
  expect(port.load).toHaveBeenCalledWith(saved.digest, expect.any(AbortSignal));
  fireEvent.click(await screen.findByRole('button', { name: 'Add Pixel artwork to layout draft' }));
  expect(await screen.findByText(/Paint floor/)).toBeTruthy();
  expect(screen.queryByText(/Added to the layout draft/)).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: 'Next library page' }));
  await waitFor(() => expect(port.list).toHaveBeenCalledWith('page2', expect.any(AbortSignal)));
  expect(screen.queryByRole('button', { name: 'Add Pixel artwork to layout draft' })).toBeNull();
  expect(port.install).not.toHaveBeenCalled();
});

it('changing canvas size requires an explicit replacement and cannot delete saved art', async () => {
  const port = fixture();
  render(<PixelWorkshop port={port} canAdd={false} add={vi.fn()} />);
  await screen.findByText('No custom artwork on this page.');
  fireEvent.keyDown(screen.getByRole('group', { name: 'Pixel drawing canvas' }), { key: ' ' });
  fireEvent.click(screen.getByRole('button', { name: 'New drawing' }));
  fireEvent.change(screen.getByLabelText('New canvas size'), { target: { value: '32' } });
  expect(screen.getByRole('group', { name: 'Pixel drawing canvas' }).getAttribute('viewBox')).toBe(
    '0 0 16 16'
  );
  fireEvent.click(screen.getByRole('button', { name: 'Replace drawing' }));
  expect(screen.getByRole('group', { name: 'Pixel drawing canvas' }).getAttribute('viewBox')).toBe(
    '0 0 32 32'
  );
  expect((screen.getByRole('button', { name: 'Undo pixels' }) as HTMLButtonElement).disabled).toBe(
    true
  );
  await act(async () => {});
  expect(port.install).not.toHaveBeenCalled();
});
