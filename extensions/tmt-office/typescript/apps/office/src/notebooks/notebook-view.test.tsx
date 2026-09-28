import { act, fireEvent, render, screen } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { NotebookView } from './notebook-view.js';
import type { Notebook } from './notebook-contract.js';
import { LocalHttpError } from '../local/local-runtime.js';

const identityId = '11111111-1111-4111-8111-111111111111';
const notebook: Notebook = {
  identityId,
  name: 'Alice',
  content: '# Private notes\n<img src="https://invalid.test/track" />\n<script>unsafe()</script>',
};

it('shows inert exact text and explicitly refreshes, clearing old content on failure', async () => {
  const read = vi.fn().mockResolvedValue(notebook);
  const view = render(<NotebookView identityId={identityId} port={{ read }} />);
  expect((await screen.findByLabelText('Notebook content')).textContent).toBe(notebook.content);
  expect(view.container.querySelector('img, script, a')).toBeNull();
  expect(read).toHaveBeenCalledTimes(1);
  read.mockRejectedValueOnce(new LocalHttpError(404, 'NOTEBOOK_IDENTITY_NOT_FOUND'));
  fireEvent.click(screen.getByRole('button', { name: 'Refresh notebook' }));
  expect((await screen.findByRole('alert')).textContent).toContain('no longer active');
  expect(screen.queryByLabelText('Notebook content')).toBeNull();
  read.mockResolvedValueOnce({ ...notebook, content: 'Updated by the agent' });
  fireEvent.click(screen.getByRole('button', { name: 'Refresh notebook' }));
  expect((await screen.findByLabelText('Notebook content')).textContent).toBe(
    'Updated by the agent'
  );
  expect(read).toHaveBeenCalledTimes(3);
});

it('aborts on close and ignores late results after switching identities', async () => {
  let resolve!: (value: Notebook) => void;
  const read = vi.fn(
    (_id: string, _signal?: AbortSignal) =>
      new Promise<Notebook>((done) => {
        resolve = done;
      })
  );
  const port = { read };
  const view = render(<NotebookView key={identityId} identityId={identityId} port={port} />);
  const first = resolve;
  const signal = read.mock.calls[0]![1]!;
  const next = '22222222-2222-4222-8222-222222222222';
  view.rerender(<NotebookView key={next} identityId={next} port={port} />);
  expect(signal.aborted).toBe(true);
  await act(async () => first(notebook));
  expect(screen.queryByLabelText('Notebook content')).toBeNull();
  await act(async () => resolve({ identityId: next, name: 'Bob', content: 'Bob only' }));
  expect(screen.getByLabelText('Notebook content').textContent).toBe('Bob only');
  view.unmount();
  expect(read.mock.calls[1]![1]!.aborted).toBe(true);
});

it('describes a missing notebook without offering implicit creation', async () => {
  const read = vi.fn().mockRejectedValue(new LocalHttpError(404, 'NOTEBOOK_NOT_FOUND'));
  render(<NotebookView identityId={identityId} port={{ read }} />);
  expect((await screen.findByRole('alert')).textContent).toContain('tmt notes path');
  expect(screen.getAllByRole('button')).toHaveLength(1);
  expect(read).toHaveBeenCalledTimes(1);
});
