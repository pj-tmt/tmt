import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import { SnapshotReferenceView } from './snapshot-reference-view.js';

const id = '11111111-1111-4111-8111-111111111111';
const reference = `tmt:whiteboard:snapshot:${id}`;

it('copies only a token-free local reference', async () => {
  const user = userEvent.setup();
  render(<SnapshotReferenceView id={id} />);
  await user.click(screen.getByRole('button', { name: 'Copy reference' }));
  expect(await navigator.clipboard.readText()).toBe(reference);
  expect(screen.getByRole('status').textContent).toBe('Reference copied.');
});

it('selects the same reference for manual copying if clipboard access is denied', async () => {
  const user = userEvent.setup();
  vi.spyOn(navigator.clipboard, 'writeText').mockRejectedValue(new Error('denied'));
  render(<SnapshotReferenceView id={id} />);
  await user.click(screen.getByRole('button', { name: 'Copy reference' }));
  const field = screen.getByRole('textbox', {
    name: 'Local snapshot reference',
  }) as HTMLInputElement;
  expect(field.readOnly).toBe(true);
  expect(field.value).toBe(reference);
  expect(field.selectionStart).toBe(0);
  expect(field.selectionEnd).toBe(reference.length);
  expect(document.activeElement).toBe(field);
  expect(screen.getByRole('status').textContent).toContain('Copy the selected reference manually');
});
