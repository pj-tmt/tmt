import { CanvasSource, Texture } from 'pixi.js';
import { indexedRuns } from './indexed-art.js';
import type { IndexedArt } from './indexed-art.js';
import { INSET_TEXT_FONT, insetTextGeometry } from './inset-text.js';

/** Admitted indexed art becomes one nearest-neighbor texture per scene/key. */
export function createSceneTextures() {
  const textures = new Map<string, Texture>();
  const used = new Set<string>();
  return {
    begin() {
      used.clear();
    },
    get(key: string, art: IndexedArt) {
      used.add(key);
      const existing = textures.get(key);
      if (existing) return existing;
      const canvas = document.createElement('canvas');
      canvas.width = art.pixels[0]!.length / (art.indexWidth ?? 1);
      canvas.height = art.pixels.length;
      const context = canvas.getContext('2d');
      if (!context) throw new Error('Pixel artwork could not initialize.');
      for (const { x, y, width, color } of indexedRuns(art)) {
        context.fillStyle = color;
        context.fillRect(x, y, width, 1);
      }
      if (art.text) {
        const text = art.text;
        const geometry = insetTextGeometry(text);
        context.save();
        context.beginPath();
        context.rect(text.x, text.y, text.width, text.height);
        context.clip();
        context.font = `${geometry.fontSize}px ${INSET_TEXT_FONT}`;
        context.fillStyle = text.color;
        context.textAlign = 'center';
        context.textBaseline = 'middle';
        context.fillText(text.value, geometry.x, geometry.y);
        context.restore();
      }
      const texture = new Texture({
        source: new CanvasSource({ resource: canvas, scaleMode: 'nearest' }),
      });
      textures.set(key, texture);
      return texture;
    },
    end() {
      for (const [key, texture] of textures) {
        if (!used.has(key)) {
          texture.destroy(true);
          textures.delete(key);
        }
      }
    },
    dispose() {
      for (const texture of textures.values()) texture.destroy(true);
      textures.clear();
      used.clear();
    },
  };
}
