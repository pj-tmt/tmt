import { Container, Sprite, Texture, TextureSource, TilingSprite } from 'pixi.js';
import { expect, it } from 'vitest';
import { PLATFORM_FRAMES } from './platform-art.js';
import type { PlatformTextures } from './platform-art.js';
import { drawPlatformEdge, platformContour, PLATFORM_ART_SCALE } from './scene-platform.js';
import type { WallRun } from './world-geometry.js';

const wall: WallRun = {
  x: 0,
  y: 0,
  width: 48,
  height: 1,
  axis: 'horizontal',
  open: false,
  exterior: true,
  raised: false,
  facing: 'negative',
  circulation: false,
  frontCorners: { start: true, end: true },
};

it('keeps every reviewed frame and silhouette inside its bundled source', () => {
  for (const frame of Object.values(PLATFORM_FRAMES)) {
    expect(frame.x + frame.width).toBeLessThanOrEqual(1536);
    expect(frame.y + frame.height).toBeLessThanOrEqual(1024);
    if ('outline' in frame)
      for (let i = 0; i < frame.outline.length; i += 2) {
        expect(frame.outline[i]).toBeGreaterThanOrEqual(0);
        expect(frame.outline[i]).toBeLessThanOrEqual(frame.width);
        expect(frame.outline[i + 1]).toBeGreaterThanOrEqual(0);
        expect(frame.outline[i + 1]).toBeLessThanOrEqual(frame.height);
      }
  }
});

it('extends Lobby repeats without stretching hardware or introducing interactive nodes', () => {
  const source = new TextureSource({ width: 1536, height: 1024 });
  const textures = Object.fromEntries(
    Object.keys(PLATFORM_FRAMES).map((key) => [key, new Texture({ source })])
  ) as PlatformTextures;
  const parent = new Container();
  try {
    const office = drawPlatformEdge(parent, wall, { x: 0, y: 0, width: 48, height: 6 }, textures);
    const lobby = drawPlatformEdge(
      parent,
      { ...wall, width: 96 },
      { x: 0, y: 60, width: 96, height: 6 },
      textures
    );
    const hardware = (group: Container) =>
      group.children.filter(
        (child) => child instanceof Sprite && !(child instanceof TilingSprite)
      ) as Sprite[];
    expect(hardware(lobby).length).toBeGreaterThan(hardware(office).length);
    for (const sprite of [...hardware(office), ...hardware(lobby)]) {
      expect(Math.abs(sprite.scale.x)).toBe(PLATFORM_ART_SCALE);
      expect(sprite.scale.y).toBe(PLATFORM_ART_SCALE);
    }
    expect(office.eventMode).toBe('none');
    expect(lobby.eventMode).toBe('none');
    const meeting = drawPlatformEdge(
      parent,
      wall,
      { x: 0, y: 0, width: 48, height: 6 },
      textures,
      undefined,
      true
    );
    expect(
      hardware(meeting).map((sprite) => ({ texture: sprite.texture, x: sprite.x, y: sprite.y }))
    ).toEqual(
      hardware(office).map((sprite) => ({
        texture:
          sprite.texture === textures.lamp
            ? textures.meetingLamp
            : sprite.texture === textures.corner
              ? textures.meetingCorner
              : sprite.texture,
        x: sprite.x,
        y: sprite.y,
      }))
    );
    const side = drawPlatformEdge(
      parent,
      { ...wall, axis: 'vertical' },
      { x: 0, y: 0, width: 1.5, height: 48 },
      textures
    );
    expect(side.children).toHaveLength(2);
    expect(hardware(side)).toHaveLength(0);
    const opening = drawPlatformEdge(
      parent,
      { ...wall, open: true },
      { x: -0.9, y: -1.806, width: 9.8, height: 4.312 },
      textures
    );
    expect(opening.children).toHaveLength(2);
    expect(opening.children[1]!.eventMode).toBe('none');
    expect(opening.children[1]!.filters ?? []).toHaveLength(0);
    const dock = opening.children[0] as Sprite;
    expect(dock.texture).toBe(textures.bridgeDock);
    expect(dock.width).toBeCloseTo(9.8);
    const bridge = drawPlatformEdge(
      parent,
      { ...wall, axis: 'vertical', circulation: true },
      { x: 19.1, y: 0, width: 1.8, height: 24 },
      textures
    );
    const rail = bridge.children[1] as TilingSprite;
    expect(rail.x + rail.width / 2).toBeCloseTo(0.9);
    expect(bridge.children[0]!.eventMode).toBe('none');
    expect(bridge.children).toHaveLength(2);
    expect(bridge.children[0]!.scale.x).toBe(1);
    const opposite = drawPlatformEdge(
      parent,
      { ...wall, axis: 'vertical', circulation: true, facing: 'positive' },
      { x: 0, y: 0, width: 1.8, height: 24 },
      textures
    );
    expect(opposite.children[0]!.scale.x).toBe(-1);
    expect(opposite.children[1]!.x).toBeCloseTo(rail.x);
  } finally {
    parent.destroy({ children: true });
    for (const texture of Object.values(textures)) texture.destroy(false);
    source.destroy();
  }
});

it('uses a beveled contour with fixed corner size for both office and Lobby', () => {
  for (const width of [48, 96]) {
    const outline = platformContour({ x: 10, y: 20, width, height: width });
    expect(outline).toHaveLength(16);
    expect(outline.slice(0, 4)).toEqual([11.5, 20, 10 + width - 1.5, 20]);
    expect(outline.slice(-4)).toEqual([10, 20 + width - 1.5, 10, 21.5]);
  }
});
