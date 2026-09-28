import { ImageSource, Rectangle, Texture } from 'pixi.js';
import architectureUrl from './assets/modular-architecture-v1.png';
import { ARCHITECTURE_FRAMES, ARCHITECTURE_SOURCE_SIZE } from './architecture-art.js';
import type { ArchitectureFrame } from './architecture-art.js';
import { recolorArchitecture } from './architecture-material.js';
import type { ModuleMaterial } from '../world-map/module-contract.js';

type MaterialFrames = Record<ArchitectureFrame, Texture>;

/** Fixed bundled architecture, decoded once per scene and shared by every room. */
export async function createSceneMaterials(signal: AbortSignal) {
  if (signal.aborted) return undefined;
  const image = new Image();
  image.src = architectureUrl;
  await image.decode();
  if (signal.aborted) return undefined;
  if (
    image.naturalWidth !== ARCHITECTURE_SOURCE_SIZE ||
    image.naturalHeight !== ARCHITECTURE_SOURCE_SIZE
  )
    throw new Error('The bundled architecture image does not match its reviewed frames.');
  const owned: Texture[] = [];
  const frames: Texture[] = [];
  const variants = new Map<ModuleMaterial, MaterialFrames>();
  function dispose() {
    variants.clear();
    for (const frame of frames.splice(0)) frame.destroy(false);
    for (const texture of owned.splice(0)) texture.destroy(true);
  }
  function frameViews(resource: HTMLImageElement | HTMLCanvasElement): MaterialFrames {
    const source = new Texture({ source: new ImageSource({ resource, scaleMode: 'nearest' }) });
    owned.push(source);
    const textures = {} as MaterialFrames;
    for (const key of Object.keys(ARCHITECTURE_FRAMES) as ArchitectureFrame[]) {
      const { x, y, width, height } = ARCHITECTURE_FRAMES[key];
      const texture = new Texture({
        source: source.source,
        frame: new Rectangle(x, y, width, height),
      });
      frames.push(texture);
      textures[key] = texture;
    }
    return textures;
  }
  try {
    const workshop = frameViews(image);
    variants.set('workshop', workshop);
    function forMaterial(material: ModuleMaterial): MaterialFrames {
      if (signal.aborted || !owned.length) throw new Error('The scene materials were disposed.');
      const cached = variants.get(material);
      if (cached) return cached;
      const canvas = document.createElement('canvas');
      canvas.width = canvas.height = ARCHITECTURE_SOURCE_SIZE;
      const context = canvas.getContext('2d', { willReadFrequently: true });
      if (!context) throw new Error('The room material could not initialize.');
      context.drawImage(image, 0, 0);
      const data = context.getImageData(0, 0, canvas.width, canvas.height);
      // Workshop is already cached. Other finishes share its reviewed frames,
      // source dimensions and exact transparent openings.
      if (material !== 'workshop') recolorArchitecture(data.data, canvas.width, material);
      context.putImageData(data, 0, 0);
      const result = frameViews(canvas);
      variants.set(material, result);
      return result;
    }
    return { ...workshop, forMaterial, dispose };
  } catch (error) {
    dispose();
    throw error;
  }
}
