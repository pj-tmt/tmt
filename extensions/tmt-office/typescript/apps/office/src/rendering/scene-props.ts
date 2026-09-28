import { Container, Graphics, Sprite } from 'pixi.js';
import { footprint } from '../blocks/block-contract.js';
import type { Furniture } from '../blocks/block-contract.js';
import { propFrame, resolvePlacedProp } from '../props/prop-contract.js';
import type { CatalogPack } from '../props/prop-contract.js';
import type { createSceneTextures } from './scene-textures.js';

/** Decorations and functional instances share the same admitted prop projection. */
export function drawSceneProp(
  parent: Container,
  item: Furniture,
  catalog: CatalogPack[],
  textures: ReturnType<typeof createSceneTextures>
) {
  const resolved = resolvePlacedProp(catalog, item);
  const size = footprint(item);
  if (!resolved) {
    parent.addChild(
      new Graphics()
        .rect(item.x, item.y, size.width, size.height)
        .fill({ color: '#9e6849', alpha: 0.4 })
        .stroke({ color: '#f5d890', width: 0.15 })
    );
    return;
  }
  const frame = propFrame(resolved.pack, resolved.definition, item.rotation, item.customization);
  const variant = JSON.stringify([
    item.prop,
    frame.frame,
    item.customization?.tint,
    item.customization?.text,
  ]);
  const sprite = new Sprite(textures.get(variant, frame));
  sprite.anchor.set(0.5);
  sprite.width = frame.width;
  sprite.height = frame.height;
  sprite.position.set(item.x + size.width / 2, item.y + size.height / 2);
  sprite.rotation = (frame.rotation * Math.PI) / 2;
  parent.addChild(sprite);
}
