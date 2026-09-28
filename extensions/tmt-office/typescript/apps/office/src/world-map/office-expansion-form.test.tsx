import { fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { OfficeExpansionForm } from './office-expansion-form.js';

const disconnect = vi.fn();
beforeEach(() => {
  vi.stubGlobal(
    'ResizeObserver',
    class {
      observe() {}
      disconnect = disconnect;
    }
  );
});
afterEach(() => {
  vi.unstubAllGlobals();
  vi.clearAllMocks();
});
const selected = { type: 'office', column: 0, row: 3 } as const;

it('focuses the name, keeps location expandable and delegates one explicit creation', () => {
  const create = vi.fn(),
    cancel = vi.fn();
  const mounted = render(
    <OfficeExpansionForm selected={selected} busy={false} create={create} cancel={cancel} />
  );
  const name = screen.getByRole('textbox', { name: 'Name' }) as HTMLInputElement;
  expect(document.activeElement).toBe(name);
  expect(screen.queryByRole('combobox')).toBeNull();
  expect(screen.getByRole('button', { name: 'Add office' }).hasAttribute('disabled')).toBe(true);
  fireEvent.change(name, { target: { value: 'Workshop' } });
  fireEvent.submit(screen.getByRole('form', { name: 'New office' }));
  expect(create).toHaveBeenCalledExactlyOnceWith('Workshop');
  fireEvent.keyDown(name, { key: 'Escape' });
  expect(cancel).toHaveBeenCalledOnce();
  mounted.unmount();
  expect(disconnect).toHaveBeenCalledOnce();
});

it('blocks creation and Escape while the shared editor is busy', () => {
  const create = vi.fn(),
    cancel = vi.fn();
  render(<OfficeExpansionForm selected={selected} busy create={create} cancel={cancel} />);
  fireEvent.submit(screen.getByRole('form', { name: 'New office' }));
  fireEvent.keyDown(screen.getByRole('textbox', { name: 'Name' }), { key: 'Escape' });
  expect(create).not.toHaveBeenCalled();
  expect(cancel).not.toHaveBeenCalled();
});
