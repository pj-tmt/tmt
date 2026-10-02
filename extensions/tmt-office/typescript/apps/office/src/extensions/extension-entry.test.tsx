import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vite-plus/test';
import definitionDocument from '../../../../../contracts/discussion-extension-v1.json';
import instanceDocument from '../../../../../contracts/lobby-extension-v1.json';
import { defaultCatalog } from '../blocks/block-contract.js';
import { bindExtension } from './extension-binding.js';
import { decodeExtensionDefinition, decodeExtensionInstance } from './extension-contract.js';
import { ExtensionEntry } from './extension-entry.js';

it('keeps a compact visible name and an explicit keyboard action through the same binding', async () => {
  const entry = bindExtension(
    decodeExtensionInstance(instanceDocument),
    [decodeExtensionDefinition(definitionDocument)],
    defaultCatalog(),
    { 'discussion.open': vi.fn() }
  );
  const activate = vi.fn();
  const focus = vi.fn();
  render(<ExtensionEntry entry={entry} activate={activate} focus={focus} />);
  const button = screen.getByRole('button', { name: 'Open discussion board' });
  expect(button.textContent).toBe('↗Discussion board');
  await userEvent.tab();
  expect(document.activeElement).toBe(button);
  expect(focus).toHaveBeenCalledWith(entry.instance.id);
  await userEvent.keyboard('{Enter}');
  expect(activate).toHaveBeenCalledExactlyOnceWith(entry.instance.id);
  await userEvent.tab();
  expect(focus).toHaveBeenLastCalledWith();
});

it('shows the unavailable reason and cannot dispatch an unregistered capability', async () => {
  const entry = bindExtension(
    decodeExtensionInstance(instanceDocument),
    [decodeExtensionDefinition(definitionDocument)],
    defaultCatalog(),
    {}
  );
  const activate = vi.fn();
  render(<ExtensionEntry entry={entry} activate={activate} />);
  const button = screen.getByRole('button', {
    name: /Discussion board — This host does not provide the action/,
  });
  expect((button as HTMLButtonElement).disabled).toBe(true);
  expect(button.textContent).toContain('This host does not provide the action.');
  await userEvent.click(button);
  expect(activate).not.toHaveBeenCalled();
});
