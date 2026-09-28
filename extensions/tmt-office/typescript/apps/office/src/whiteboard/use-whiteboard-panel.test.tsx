import { fireEvent, render, screen } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { useWhiteboardPanel } from './use-whiteboard-panel.js';
import type { WhiteboardLeaveState } from './editor-state.js';

// Exercise panel lifetime and target admission separately from canvas editing.
// Native browser coverage uses the real editor, storage and failed-save transport.
vi.mock('./editor.js', () => ({
  WhiteboardEditor: ({
    documentId,
    onLeaveStateChange,
  }: {
    documentId: string;
    onLeaveStateChange(state: WhiteboardLeaveState): void;
  }) => (
    <section aria-label={`Editor ${documentId}`}>
      <input aria-label="Retained draft" defaultValue="" />
      <button onClick={() => onLeaveStateChange('draft')}>Make draft</button>
      <button onClick={() => onLeaveStateChange('pending')}>Unconfirmed save</button>
      <button onClick={() => onLeaveStateChange('ready')}>Save confirmed</button>
    </section>
  ),
}));

function Host() {
  const board = useWhiteboardPanel();
  return (
    <>
      <button onClick={() => board.open('lobby')}>Lobby board</button>
      <button onClick={() => board.open('design')}>Design board</button>
      {board.panel}
    </>
  );
}
const click = (name: string) => fireEvent.click(screen.getByRole('button', { name }));

it('retains the same resource across closing and requires explicit discard for a different resource', () => {
  render(<Host />);
  click('Lobby board');
  const input = screen.getByRole('textbox', { name: 'Retained draft' });
  fireEvent.change(input, { target: { value: 'Do not lose this' } });
  click('Make draft');
  click('Close whiteboard');
  click('Lobby board');
  expect(screen.getByRole('textbox')).toBe(input);
  click('Design board');
  expect(screen.getByRole('region', { name: 'Switch whiteboard confirmation' })).toBeTruthy();
  expect(screen.getByRole('region', { name: 'Editor lobby' })).toBeTruthy();
  click('Keep this whiteboard');
  expect(screen.getByRole('textbox')).toBe(input);
  click('Design board');
  click('Discard draft and switch');
  expect(screen.getByRole('region', { name: 'Editor design' })).toBeTruthy();
  expect(screen.getByRole('textbox')).not.toBe(input);
});

it('blocks abandoning an uncertain save, including when uncertainty starts during confirmation', () => {
  render(<Host />);
  click('Lobby board');
  click('Make draft');
  click('Design board');
  click('Unconfirmed save');
  const discard = screen.getByRole('button', { name: 'Discard draft and switch' });
  expect((discard as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(discard);
  expect(screen.getByRole('region', { name: 'Editor lobby' })).toBeTruthy();
  click('Close whiteboard');
  click('Design board');
  expect(
    (screen.getByRole('button', { name: 'Discard draft and switch' }) as HTMLButtonElement).disabled
  ).toBe(true);
  click('Keep this whiteboard');
  click('Save confirmed');
  click('Design board');
  expect(screen.getByRole('region', { name: 'Editor design' })).toBeTruthy();
  expect(screen.queryByRole('region', { name: 'Switch whiteboard confirmation' })).toBeNull();
});
