import { validFurniture, validPlacement } from '../blocks/block-contract.js';
import type { Furniture } from '../blocks/block-contract.js';
import { resolvePlacedProp } from '../props/prop-contract.js';
import type { CatalogPack } from '../props/prop-contract.js';
import type {
  ExtensionDefinition,
  ExtensionInstance,
  WorldCapability,
  ResourceBinding,
} from './extension-contract.js';
import type { WorldObject } from '../world-map/world-contract.js';
import { decodeResourceBinding } from './extension-contract.js';

export type WorldHandlers = Partial<Record<WorldCapability, (binding: ResourceBinding) => void>>;
type ExtensionTarget = Pick<
  ExtensionInstance,
  'id' | 'definition' | 'binding' | 'x' | 'y' | 'rotation'
>;
export interface BoundExtension {
  instance: ExtensionTarget;
  label: string;
  appearance?: Furniture;
  actionLabel?: string;
  unavailable?: string;
  invoke?: () => void;
}

/** Compose admitted declarations with host-owned capabilities; never infer a grant. */
export function bindExtension(
  instance: ExtensionInstance,
  definitions: readonly ExtensionDefinition[],
  catalog: CatalogPack[],
  handlers: WorldHandlers
): BoundExtension {
  const definition = definitions.find((value) => value.id === instance.definition);
  const appearance = definition && {
    ...definition.appearance,
    x: instance.x,
    y: instance.y,
    rotation: instance.rotation,
  };
  return composeExtension(
    instance,
    definition,
    validFurniture(appearance) ? appearance : undefined,
    catalog,
    handlers
  );
}

/** A saved object's placement and skin are independent of its resource reference. */
export function bindWorldExtension(
  object: WorldObject,
  definitions: readonly ExtensionDefinition[],
  catalog: CatalogPack[],
  handlers: WorldHandlers
): BoundExtension | undefined {
  if (!object.extension) return undefined;
  const instance: ExtensionTarget = {
    ...object.extension,
    id: object.id,
    x: object.placement.x,
    y: object.placement.y,
    rotation: object.placement.rotation,
  };
  return composeExtension(
    instance,
    definitions.find((value) => value.id === instance.definition),
    validPlacement(object.placement) ? object.placement : undefined,
    catalog,
    handlers
  );
}

function composeExtension(
  instance: ExtensionTarget,
  definition: ExtensionDefinition | undefined,
  appearance: Furniture | undefined,
  catalog: CatalogPack[],
  handlers: WorldHandlers
): BoundExtension {
  const entry = {
    instance,
    label: definition?.label ?? instance.definition,
    appearance,
    actionLabel: definition?.action.label,
  };
  if (!definition) return { ...entry, unavailable: 'Extension unavailable.' };
  try {
    decodeResourceBinding(instance.binding);
  } catch {
    return { ...entry, unavailable: 'Resource binding is invalid.' };
  }
  if (!appearance)
    return {
      ...entry,
      appearance: undefined,
      unavailable: 'Placement is invalid for its surface.',
    };
  if (definition.action.resourceKind !== instance.binding.kind)
    return { ...entry, unavailable: 'Resource binding is incompatible.' };
  if (!resolvePlacedProp(catalog, appearance))
    return { ...entry, unavailable: 'Appearance is unavailable or incompatible.' };
  const handler = handlers[definition.action.capability];
  if (!handler) return { ...entry, unavailable: 'This host does not provide the action.' };
  return { ...entry, invoke: () => handler(instance.binding) };
}

/** The same guarded dispatch is used by canvas and accessible controls. */
export function activateExtension(entries: readonly BoundExtension[], id: string) {
  const entry = entries.find((value) => value.instance.id === id);
  if (!entry?.invoke || entry.unavailable) return false;
  entry.invoke();
  return true;
}
