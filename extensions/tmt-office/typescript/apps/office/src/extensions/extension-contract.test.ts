import { expect, it, vi } from 'vite-plus/test';
import vectors from '../../../../../contracts/extension-vectors.json';
import pairs from '../../../../../contracts/extension-pair-vectors.json';
import definitionDocument from '../../../../../contracts/discussion-extension-v1.json';
import instanceDocument from '../../../../../contracts/lobby-extension-v1.json';
import whiteboardDefinition from '../../../../../contracts/whiteboard-extension-v1.json';
import whiteboardInstance from '../../../../../contracts/lobby-whiteboard-v1.json';
import broadcasterDefinition from '../../../../../contracts/broadcaster-extension-v1.json';
import broadcasterInstance from '../../../../../contracts/lobby-broadcaster-v1.json';
import notebookVectors from '../../../../../contracts/notebook-binding-vectors.json';
import notebookDefinition from '../../../../../contracts/notebook-extension-v1.json';
import { defaultCatalog } from '../blocks/block-contract.js';
import {
  decodeExtensionDefinition,
  decodeExtensionInstance,
  decodeResourceBinding,
} from './extension-contract.js';
import { activateExtension, bindExtension } from './extension-binding.js';

it.each(notebookVectors)('notebook binding: $name', ({ binding, valid }) => {
  if (valid) expect(decodeResourceBinding(binding)).toEqual(binding);
  else expect(() => decodeResourceBinding(binding)).toThrow();
});

it('notebook admission does not read notes; only its registered capability can open them', () => {
  const definition = decodeExtensionDefinition(notebookDefinition);
  const instance = decodeExtensionInstance({
    ...instanceDocument,
    definition: definition.id,
    binding: notebookVectors[0]!.binding,
  });
  const read = vi.fn();
  const entry = bindExtension(instance, [definition], defaultCatalog(), { 'notebook.open': read });
  expect(read).not.toHaveBeenCalled();
  expect(activateExtension([entry], instance.id)).toBe(true);
  expect(read).toHaveBeenCalledExactlyOnceWith(instance.binding);
  const unavailable = bindExtension(instance, [definition], defaultCatalog(), {
    'discussion.open': vi.fn(),
  });
  expect(activateExtension([unavailable], instance.id)).toBe(false);
  expect(read).toHaveBeenCalledTimes(1);
});

it.each(pairs.cases)(
  'paired binding agrees with native preflight: $name',
  ({ instance, problem }) => {
    const handler = vi.fn();
    const entry = bindExtension(
      decodeExtensionInstance(instance),
      [decodeExtensionDefinition(pairs.definition)],
      defaultCatalog(),
      { 'discussion.open': handler }
    );
    expect(Boolean(entry.invoke)).toBe(problem === null);
    expect(handler).not.toHaveBeenCalled();
  }
);

it.each(vectors.definitions)('definition: $name', ({ value, valid }) => {
  if (valid) expect(decodeExtensionDefinition(value)).toEqual(value);
  else expect(() => decodeExtensionDefinition(value)).toThrow();
});
it.each(vectors.instances)('instance: $name', ({ value, valid }) => {
  if (valid) expect(decodeExtensionInstance(value)).toEqual(value);
  else expect(() => decodeExtensionInstance(value)).toThrow();
});

it('opening requires a registered handler; admission and rendering do not execute it', () => {
  const definition = decodeExtensionDefinition(definitionDocument);
  const instance = decodeExtensionInstance(instanceDocument);
  const original = JSON.stringify({ definition, instance });
  const handler = vi.fn();
  const entry = bindExtension(instance, [definition], defaultCatalog(), {
    'discussion.open': handler,
  });
  expect(entry.unavailable).toBeUndefined();
  expect(handler).not.toHaveBeenCalled();
  expect(activateExtension([entry], 'not-an-instance')).toBe(false);
  expect(activateExtension([entry], instance.id)).toBe(true);
  expect(handler).toHaveBeenCalledExactlyOnceWith({ kind: 'office-board' });
  const second = bindExtension(
    { ...instance, id: 'another-board' },
    [definition],
    defaultCatalog(),
    { 'discussion.open': handler }
  );
  expect(activateExtension([second], second.instance.id)).toBe(true);
  expect(handler).toHaveBeenLastCalledWith(instance.binding);
  expect(JSON.stringify({ definition, instance })).toBe(original);
});

it('missing definitions, art, handlers and invalid combined placement remain inert', () => {
  const definition = decodeExtensionDefinition(definitionDocument);
  const instance = decodeExtensionInstance(instanceDocument);
  const handler = vi.fn();
  const entries = [
    bindExtension(instance, [], defaultCatalog(), { 'discussion.open': handler }),
    bindExtension(instance, [definition], [], { 'discussion.open': handler }),
    bindExtension(instance, [definition], defaultCatalog(), {}),
    bindExtension({ ...instance, x: 31 }, [definition], defaultCatalog(), {
      'discussion.open': handler,
    }),
  ];
  for (const entry of entries) {
    expect(entry.unavailable).toEqual(expect.any(String));
    expect(entry.invoke).toBeUndefined();
    expect(activateExtension([entry], entry.instance.id)).toBe(false);
    expect(entry.instance.binding).toEqual({ kind: 'office-board' });
  }
  expect(entries[1]!.appearance?.prop).toBe(definition.appearance.prop);
  expect(handler).not.toHaveBeenCalled();
  // Removing an instance changes only the collection, never its retained resource reference.
  expect(activateExtension([], instance.id)).toBe(false);
  expect(instance.binding).toEqual({ kind: 'office-board' });
});

it('routes whiteboards only to the matching host capability and preserves the resource ID', () => {
  const definition = decodeExtensionDefinition(whiteboardDefinition);
  const instance = decodeExtensionInstance(whiteboardInstance);
  const open = vi.fn();
  const discussion = vi.fn();
  const handlers = { 'whiteboard.open': open, 'discussion.open': discussion };
  const entry = bindExtension(instance, [definition], defaultCatalog(), handlers);
  expect(open).not.toHaveBeenCalled();
  expect(activateExtension([entry], instance.id)).toBe(true);
  expect(open).toHaveBeenCalledExactlyOnceWith({ kind: 'whiteboard', documentId: 'lobby' });
  expect(discussion).not.toHaveBeenCalled();
  const mismatched = bindExtension(
    { ...instance, binding: { kind: 'office-board' } },
    [definition],
    defaultCatalog(),
    handlers
  );
  expect(mismatched.unavailable).toBe('Resource binding is incompatible.');
  expect(activateExtension([mismatched], instance.id)).toBe(false);
  const unsupported = bindExtension(instance, [definition], defaultCatalog(), {
    'discussion.open': discussion,
  });
  expect(activateExtension([unsupported], instance.id)).toBe(false);
  expect(open).toHaveBeenCalledTimes(1);
  expect(discussion).not.toHaveBeenCalled();
});

it('opens the broadcaster only through its own registered typed binding', () => {
  const definition = decodeExtensionDefinition(broadcasterDefinition);
  const instance = decodeExtensionInstance(broadcasterInstance);
  const open = vi.fn();
  const discussion = vi.fn();
  const handlers = { 'broadcast.open': open, 'discussion.open': discussion };
  const entry = bindExtension(instance, [definition], defaultCatalog(), handlers);
  expect(open).not.toHaveBeenCalled();
  expect(activateExtension([entry], instance.id)).toBe(true);
  expect(open).toHaveBeenCalledExactlyOnceWith({ kind: 'office-broadcast' });
  for (const invalid of [
    bindExtension(instance, [definition], defaultCatalog(), { 'discussion.open': discussion }),
    bindExtension(
      { ...instance, binding: { kind: 'office-board' } },
      [definition],
      defaultCatalog(),
      handlers
    ),
  ])
    expect(activateExtension([invalid], instance.id)).toBe(false);
  expect(discussion).not.toHaveBeenCalled();
  expect(open).toHaveBeenCalledTimes(1);
});
