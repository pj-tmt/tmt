import { Container, Texture, TextureSource, TilingSprite } from 'pixi.js';
import { expect, it } from 'vitest';
import { BRIDGE_DECK_SCALE, drawBridgeDeck } from './scene-skybridge.js';
import { FLOOR_DEPTH } from './world-projection.js';

it('tiles narrow metal panels at fixed scale and keeps neighboring floor runs aligned', () => {
  const texture = new Texture({ source: new TextureSource({ width: 71, height: 128 }) });
  const parent = new Container();
  try {
    for (const y of [10, 11]) drawBridgeDeck(parent, { x: 20, y, width: 8, height: 1 }, texture);
    for (const child of parent.children) {
      const deck = child as TilingSprite;
      expect(deck.tileScale.x).toBe(BRIDGE_DECK_SCALE);
      expect(deck.tileScale.y).toBe(BRIDGE_DECK_SCALE * FLOOR_DEPTH);
      expect(deck.x + deck.tilePosition.x).toBe(0);
      expect(deck.y + deck.tilePosition.y).toBe(0);
      expect(deck.eventMode).toBe('none');
    }
  } finally {
    parent.destroy({ children: true });
    texture.destroy(true);
  }
});
