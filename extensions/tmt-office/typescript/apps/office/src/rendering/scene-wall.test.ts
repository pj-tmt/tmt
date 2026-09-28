import {
  Container,
  Graphics,
  NineSliceSprite,
  Texture,
  TextureSource,
  TilingSprite,
} from 'pixi.js';
import { expect, it } from 'vitest';
import { drawWall } from './scene-wall.js';
import { ARCHITECTURE_FRAMES } from './architecture-art.js';
import { wallProjection } from './world-geometry.js';
import type { WallRun } from './world-geometry.js';
import { flatProjection } from './world-projection.js';

function assertAtlasSprite(value: Container): asserts value is NineSliceSprite {
  if (!(value instanceof NineSliceSprite)) throw new Error('Expected retained atlas wall art.');
}

function wall(changes: Partial<WallRun> = {}): WallRun {
  return {
    x: 20,
    y: 40,
    width: 48,
    height: 1,
    axis: 'horizontal',
    open: false,
    exterior: false,
    raised: true,
    facing: 'positive',
    circulation: false,
    ...changes,
  };
}

it('raises interior partitions exactly as high as exterior back walls', () => {
  const interior = wallProjection(wall());
  expect(interior).toEqual({
    art: 'back',
    bounds: { x: 20, y: 9, width: 48, height: 16 },
    depth: 25,
  });
  expect(wallProjection(wall({ exterior: true }))).toEqual(interior);
  expect(wallProjection(wall({ raised: false, facing: 'negative' }))).toEqual({
    art: 'sill',
    bounds: { x: 20, y: 9, width: 48, height: 16 },
    depth: 25,
  });
});

it('uses distinct authored side faces and a real portal for each opening axis', () => {
  const side = wall({ axis: 'vertical', width: 1, height: 40, raised: false });
  expect(wallProjection(side)).toEqual({
    art: 'left',
    bounds: { x: 17, y: 9, width: 6, height: 41 },
    depth: 50,
  });
  expect(wallProjection({ ...side, facing: 'negative' }).art).toBe('right');
  expect(wallProjection({ ...side, open: true }).art).toBe('sidePortal');
  expect(wallProjection(wall({ open: true, width: 8 })).art).toBe('portal');
});

it('keeps common circulation rails low without shrinking a room side wall', () => {
  const run = wall({ axis: 'vertical', width: 1, height: 8, exterior: true, circulation: true });
  expect(wallProjection(run)).toEqual({
    art: 'left',
    bounds: { x: 19, y: 21, width: 2, height: 9 },
    depth: 30,
  });
  expect(wallProjection({ ...run, circulation: false }).bounds.height).toBe(21);
  expect(wallProjection(wall({ circulation: true })).art).toBe('rail');
});

it('keeps painted bounds and reserves a source center even on short walls and doorways', () => {
  const source = new TextureSource({ width: 1254, height: 1254 });
  const texture = new Texture({ source });
  const railTexture = new Texture({ source });
  const textures = {
    back: texture,
    sill: texture,
    rail: railTexture,
    frontPost: texture,
    left: texture,
    right: texture,
    leftBody: texture,
    rightBody: texture,
    leftCrown: texture,
    rightCrown: texture,
    portal: texture,
    sidePortal: texture,
    bridgeDeck: texture,
    bridgeRail: texture,
    bridgeDock: texture,
  };
  try {
    const rail = new Container();
    try {
      const { sprite } = drawWall(rail, wall({ width: 8, circulation: true }), textures);
      assertAtlasSprite(sprite);
      expect(sprite.texture).toBe(textures.rail);
      expect(sprite.width * sprite.scale.x).toBeCloseTo(8);
      expect(sprite.height * sprite.scale.y).toBeCloseTo(4);
      expect(rail.children).toHaveLength(1);
    } finally {
      rail.destroy({ children: true });
    }
    for (const width of [4, 8, 48, 1984]) {
      const parent = new Container();
      try {
        const run = wall({ width });
        const { sprite, depth } = drawWall(parent, run, textures);
        assertAtlasSprite(sprite);
        expect(depth).toBe(25);
        expect(parent.children).toHaveLength(1);
        expect(sprite.leftWidth).toBe(0);
        expect(sprite.rightWidth).toBe(0);
        expect(sprite.position.x).toBe(20);
        expect(sprite.position.y).toBe(9);
        expect(sprite.width * sprite.scale.x).toBeCloseTo(width);
        expect(sprite.height * sprite.scale.y).toBeCloseTo(16);
        expect(sprite.width - sprite.leftWidth - sprite.rightWidth).toBeGreaterThan(0);
        // A longer wall changes mesh extents, not a loop of one display object per tile.
        expect(sprite.texture).toBe(textures.back);
      } finally {
        parent.destroy({ children: true });
      }
    }
    const door = new Container();
    try {
      const { sprite } = drawWall(door, wall({ width: 8, open: true }), textures);
      assertAtlasSprite(sprite);
      expect(sprite.width - sprite.leftWidth - sprite.rightWidth).toBeGreaterThan(100);
      expect(sprite.width * sprite.scale.x).toBeCloseTo(8);
      expect(sprite.texture).toBe(textures.portal);
    } finally {
      door.destroy({ children: true });
    }
    const front = new Container();
    try {
      const { sprite } = drawWall(
        front,
        wall({ raised: false, frontCorners: { start: true, end: true } }),
        textures
      );
      // Cutaway keeps the same physical crown and base as the full-height wall.
      assertAtlasSprite(sprite);
      expect(sprite.topHeight * sprite.scale.y).toBeCloseTo(35 * 0.08);
      expect(sprite.bottomHeight * sprite.scale.y).toBeCloseTo(21 * 0.08);
      expect(sprite.height - sprite.topHeight - sprite.bottomHeight).toBeGreaterThan(0);
      expect(sprite.height * sprite.scale.y).toBeCloseTo(16);
      expect(sprite.x).toBe(26);
      expect(sprite.width * sprite.scale.x).toBeCloseTo(36);
      expect(front.children).toHaveLength(3);
      expect(
        front.children.slice(1).map((post) => [post.x, post.y, post.width, post.height])
      ).toEqual([
        [20, 9, 6, 16],
        [62, 9, 6, 16],
      ]);
      for (const post of front.children.slice(1))
        expect(post).toHaveProperty('texture', textures.frontPost);
      expect(front.children.map((node) => node.alpha)).toEqual([1, 1, 1]);
      const editing = new Container();
      try {
        drawWall(
          editing,
          wall({ raised: false, frontCorners: { start: true, end: true } }),
          textures,
          flatProjection,
          true
        );
        expect(editing.children.map((node) => node.alpha)).toEqual([0.25, 0.5, 0.5]);
        expect(editing.children.map((node) => [node.x, node.y, node.width, node.height])).toEqual(
          front.children.map((node) => [node.x, node.y, node.width, node.height])
        );
      } finally {
        editing.destroy({ children: true });
      }
    } finally {
      front.destroy({ children: true });
    }
    for (const axis of ['horizontal', 'vertical'] as const) {
      const dock = new Container();
      try {
        const run = wall({ axis, width: 8, height: 8, open: true, circulation: true });
        const { sprite } = drawWall(dock, run, textures, { ...flatProjection, version: 6 });
        expect(sprite).toBeInstanceOf(Container);
        expect(sprite.width * sprite.height).toBe(0);
        expect(dock.children).toHaveLength(1);
        expect(sprite).not.toBeInstanceOf(NineSliceSprite);
        expect(sprite.eventMode).toBe('none');
        expect(sprite.filters ?? []).toHaveLength(0);
        expect(sprite.children.some((child) => child instanceof TilingSprite)).toBe(false);
      } finally {
        dock.destroy({ children: true });
      }
    }
    for (const axis of ['horizontal', 'vertical'] as const) {
      const bridge = new Container();
      try {
        drawWall(bridge, wall({ axis, width: 16, height: 16, circulation: true }), textures, {
          ...flatProjection,
          version: 6,
        });
        expect(bridge.children).toHaveLength(2);
        expect(bridge.children[1]).toBeInstanceOf(Graphics);
        expect(bridge.children[1]!.width).toBeGreaterThan(0);
        expect(bridge.children[1]!.eventMode).toBe('none');
        expect(bridge.children[1]!.filters ?? []).toHaveLength(0);
      } finally {
        bridge.destroy({ children: true });
      }
    }
    expect(source.destroyed).toBe(false);
  } finally {
    railTexture.destroy();
    texture.destroy(true);
  }
});

it('keeps doorway projection within the open ground interval rather than painting a solid spanning bay', () => {
  const opening = wallProjection(wall({ x: 30, width: 8, open: true }));
  const before = wallProjection(wall({ x: 20, width: 10 }));
  const after = wallProjection(wall({ x: 38, width: 30 }));
  expect(before.bounds.x + before.bounds.width).toBe(opening.bounds.x);
  expect(opening.bounds.x + opening.bounds.width).toBe(after.bounds.x);
  expect(opening.art).toBe('portal');
  expect(opening.bounds.height).toBe(16);
  expect(opening.bounds.y + opening.bounds.height).toBe(before.depth);
  expect(opening.bounds.y).toBe(before.bounds.y);
  expect(opening.bounds.height).toBe(before.bounds.height);
  expect(ARCHITECTURE_FRAMES.portal).not.toEqual(ARCHITECTURE_FRAMES.back);
});
