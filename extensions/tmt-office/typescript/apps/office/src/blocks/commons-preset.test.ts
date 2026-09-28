import { expect, it } from 'vitest';
import document from '../../../../../../../contracts/office/commons-preset-v1.json';
import discussion from '../../../../../../../contracts/office/discussion-extension-v1.json';
import discussionInstance from '../../../../../../../contracts/office/lobby-extension-v1.json';
import whiteboard from '../../../../../../../contracts/office/whiteboard-extension-v1.json';
import whiteboardInstance from '../../../../../../../contracts/office/lobby-whiteboard-v1.json';
import broadcaster from '../../../../../../../contracts/office/broadcaster-extension-v1.json';
import broadcasterInstance from '../../../../../../../contracts/office/lobby-broadcaster-v1.json';
import { defaultCatalog, footprint, validLocalLayout } from './block-contract.js';
import { COMMONS_FURNITURE } from './commons-preset.js';
import { resolvePlacedProp } from '../props/prop-contract.js';
import { bindExtension } from '../extensions/extension-binding.js';
import {
  decodeExtensionDefinition,
  decodeExtensionInstance,
} from '../extensions/extension-contract.js';

it('uses admitted, serializable furniture and declared customization without adding capabilities', () => {
  expect(validLocalLayout(document)).toBe(true);
  expect(COMMONS_FURNITURE).toEqual(JSON.parse(JSON.stringify(document)).objects);
  expect(COMMONS_FURNITURE).toHaveLength(8);
  for (const item of COMMONS_FURNITURE) {
    expect(resolvePlacedProp(defaultCatalog(), item), item.prop).toBeDefined();
  }
  expect(COMMONS_FURNITURE.filter((item) => item.customization)).toEqual([
    expect.objectContaining({ customization: { text: 'Better together' } }),
  ]);
});

it('keeps upright fixtures outside the actual functional-object footprints', () => {
  const definitions = [discussion, whiteboard, broadcaster].map(decodeExtensionDefinition);
  const instances = [discussionInstance, whiteboardInstance, broadcasterInstance].map(
    decodeExtensionInstance
  );
  for (const instance of instances) {
    const entry = bindExtension(instance, definitions, defaultCatalog(), {});
    expect(entry.appearance).toBeDefined();
    const target = entry.appearance!;
    const targetSize = footprint(target);
    // Textiles intentionally underlay stations. Upright sprites must not cover
    // a functional object or suggest that clicking a chair opens a board.
    for (const item of COMMONS_FURNITURE.filter((item) => !item.prop.endsWith('/woven-rug'))) {
      const size = footprint(item);
      const overlaps =
        item.x < target.x + targetSize.width &&
        item.x + size.width > target.x &&
        item.y < target.y + targetSize.height &&
        item.y + size.height > target.y;
      expect(overlaps, `${item.prop} obscures ${instance.id}`).toBe(false);
    }
  }
});
