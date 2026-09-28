import { Graphics, TilingSprite } from 'pixi.js';
import type { Container, Texture } from 'pixi.js';
import type { SceneRect } from './office-geometry.js';
import { FLOOR_DEPTH } from './world-projection.js';

export const BRIDGE_DECK_SCALE = 0.09;

/** One continuous blue-green support plane below all deck runs. Drawing these
 * before every deck prevents a later run's base covering an earlier metal panel.
 */
export function drawBridgeBase(parent: Container, rect: SceneRect) {
  const base = new Graphics();
  base.eventMode = 'none';
  base.rect(rect.x - 0.7, rect.y + 1, rect.width + 1.4, rect.height + 1.2).fill('#0c2028');
  base.rect(rect.x - 0.7, rect.y + 0.4, rect.width + 1.4, rect.height + 1).fill('#294c54');
  base.rect(rect.x - 0.5, rect.y, rect.width + 1, rect.height).fill('#527c7d');
  parent.addChild(base);
}

/** Bridge decking shares the platform plane. Boundaries own its edge trim. */
export function drawBridgeDeck(parent: Container, rect: SceneRect, texture: Texture) {
  // A floor-board repeat spans 24 world units; using it for this narrow authored
  // deck magnifies its seams into stripes. Keep metal panels at their own scale.
  const deck = new TilingSprite({ texture, width: rect.width, height: rect.height });
  deck.position.set(rect.x, rect.y);
  deck.tileScale.set(BRIDGE_DECK_SCALE, BRIDGE_DECK_SCALE * FLOOR_DEPTH);
  deck.tilePosition.set(-rect.x, -rect.y);
  deck.eventMode = 'none';
  parent.addChild(deck);
}
