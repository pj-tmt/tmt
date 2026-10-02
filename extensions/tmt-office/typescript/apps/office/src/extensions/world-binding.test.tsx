import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vite-plus/test';
import definitionDocument from '../../../../../contracts/whiteboard-extension-v1.json';
import { builtinFurniture, defaultCatalog } from '../blocks/block-contract.js';
import { decodeExtensionDefinition } from './extension-contract.js';
import { activateExtension, bindWorldExtension } from './extension-binding.js';
import type { WorldObject } from '../world-map/world-contract.js';
import { ExtensionEntry } from './extension-entry.js';

const definition = decodeExtensionDefinition(definitionDocument);
const object: WorldObject = {
  id: '30000000-0000-4000-8000-000000000001',
  kind: 'decoration',
  placement: builtinFurniture('chair', -4, -3, 0),
  surface: { type: 'floor' },
  extension: { definition: definition.id, binding: { kind: 'whiteboard', documentId: 'lobby' } },
};

it('opens the same retained resource through either spatial activation or its accessible world-object entry', async () => {
  const open = vi.fn();
  const first = bindWorldExtension(object, [definition], defaultCatalog(), {
    'whiteboard.open': open,
  })!;
  const second = bindWorldExtension(
    {
      ...object,
      id: '30000000-0000-4000-8000-000000000002',
      placement: { ...object.placement, x: 50 },
    },
    [definition],
    defaultCatalog(),
    { 'whiteboard.open': open }
  )!;
  expect(first.appearance).toEqual(object.placement);
  expect(second.appearance?.x).toBe(50);
  expect(first.unavailable).toBeUndefined();
  expect(open).not.toHaveBeenCalled();
  expect(activateExtension([first, second], second.instance.id)).toBe(true);
  render(
    <ExtensionEntry
      entry={first}
      activate={(id) => {
        activateExtension([first, second], id);
      }}
    />
  );
  await userEvent.click(screen.getByRole('button', { name: 'Open whiteboard' }));
  expect(open).toHaveBeenCalledTimes(2);
  expect(open.mock.calls).toEqual([
    [{ kind: 'whiteboard', documentId: 'lobby' }],
    [{ kind: 'whiteboard', documentId: 'lobby' }],
  ]);
});

it('keeps unknown definitions, incompatible bindings and missing handlers inert without dropping art or resource IDs', () => {
  const open = vi.fn();
  const entries = [
    bindWorldExtension(object, [], defaultCatalog(), { 'whiteboard.open': open })!,
    bindWorldExtension(object, [definition], defaultCatalog(), {})!,
    bindWorldExtension(object, [definition], [], { 'whiteboard.open': open })!,
    bindWorldExtension(
      { ...object, extension: { definition: definition.id, binding: { kind: 'office-board' } } },
      [definition],
      defaultCatalog(),
      { 'whiteboard.open': open }
    )!,
  ];
  for (const entry of entries) {
    expect(entry.appearance).toEqual(object.placement);
    expect(entry.unavailable).toEqual(expect.any(String));
    expect(activateExtension([entry], object.id)).toBe(false);
  }
  expect(open).not.toHaveBeenCalled();
  expect(
    bindWorldExtension({ ...object, extension: null }, [definition], defaultCatalog(), {
      'whiteboard.open': open,
    })
  ).toBeUndefined();
});
