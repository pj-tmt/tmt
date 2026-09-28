import { afterEach, expect, it, vi } from 'vitest';
import { createSceneMaterials } from './scene-materials.js';
import { ARCHITECTURE_FRAMES } from './architecture-art.js';

const evidence = vi.hoisted(() => ({ decoded: [] as string[], disposed: [] as string[] }));

// Unit coverage observes resource ownership only. Chromium exercises real image
// decode, texture bounds, timber paint and teardown in the composition scenarios.
vi.mock('pixi.js', () => ({
  ImageSource: class {
    constructor(public options: { resource: { src: string } }) {}
  },
  Rectangle: class {
    constructor(
      public x: number,
      public y: number,
      public width: number,
      public height: number
    ) {}
  },
  Texture: class {
    source: { options: { resource: { src: string } } };
    frame?: unknown;
    constructor(options: { source: { options: { resource: { src: string } } }; frame?: unknown }) {
      this.source = options.source;
      this.frame = options.frame;
    }
    destroy(source: boolean) {
      evidence.disposed.push(
        this.frame ? `frame:${source}` : `${this.source.options.resource.src}:${source}`
      );
    }
  },
}));

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  evidence.decoded = [];
  evidence.disposed = [];
});

function images() {
  vi.stubGlobal(
    'Image',
    class {
      src = '';
      naturalWidth = 1254;
      naturalHeight = 1254;
      async decode() {
        evidence.decoded.push(this.src);
      }
    }
  );
}

it('shares one architecture source across reviewed frames and disposes views before the source once', async () => {
  images();
  const materials = await createSceneMaterials(new AbortController().signal);
  expect(evidence.decoded).toHaveLength(1);
  expect(materials!.back.source).toBe(materials!.floor.source);
  expect(materials!.portal.source).toBe(materials!.floor.source);
  expect(materials!.back.frame).toMatchObject(ARCHITECTURE_FRAMES.back);
  expect(materials!.floor.frame).toMatchObject(ARCHITECTURE_FRAMES.floor);
  materials!.dispose();
  expect(evidence.disposed).toEqual([
    ...Array(16).fill('frame:false'),
    `${evidence.decoded[0]}:true`,
  ]);
  materials!.dispose();
  expect(evidence.disposed).toHaveLength(17);
});

it('lazily caches at most one source per finish and releases every frame before its source', async () => {
  images();
  const context = {
    drawImage: vi.fn(),
    getImageData: vi.fn(() => ({ data: new Uint8ClampedArray(1254 * 1254 * 4) })),
    putImageData: vi.fn(),
  };
  const create = document.createElement.bind(document);
  vi.spyOn(document, 'createElement').mockImplementation((tag, options) => {
    const element = create(tag, options);
    if (tag === 'canvas') Object.defineProperty(element, 'getContext', { value: () => context });
    return element;
  });
  const materials = (await createSceneMaterials(new AbortController().signal))!;
  expect(context.drawImage).not.toHaveBeenCalled();
  expect(materials.forMaterial('workshop').floor).toBe(materials.floor);
  const moonlight = materials.forMaterial('moonlight');
  const copper = materials.forMaterial('copper');
  for (let i = 0; i < 10; i++) {
    expect(materials.forMaterial('moonlight')).toBe(moonlight);
    expect(materials.forMaterial('copper')).toBe(copper);
  }
  expect(evidence.decoded).toHaveLength(1);
  expect(context.drawImage).toHaveBeenCalledTimes(2);
  expect(context.putImageData).toHaveBeenCalledTimes(2);
  expect(moonlight.back.source).toBe(moonlight.floor.source);
  expect(moonlight.floor.source).not.toBe(materials.floor.source);
  expect(copper.floor.source).not.toBe(moonlight.floor.source);
  expect(moonlight.portal.frame).toMatchObject(ARCHITECTURE_FRAMES.portal);
  materials.dispose();
  expect(evidence.disposed.slice(0, 48)).toEqual(Array(48).fill('frame:false'));
  expect(evidence.disposed).toHaveLength(51);
  materials.dispose();
  expect(evidence.disposed).toHaveLength(51);
  expect(() => materials.forMaterial('copper')).toThrow('disposed');
});

it('rejects an unexpected sheet before allocating GPU ownership or silently accepting wrong crops', async () => {
  vi.stubGlobal(
    'Image',
    class {
      src = '';
      naturalWidth = 2048;
      naturalHeight = 2048;
      async decode() {}
    }
  );
  await expect(createSceneMaterials(new AbortController().signal)).rejects.toThrow(
    'reviewed frames'
  );
  expect(evidence.disposed).toEqual([]);
});

it('creates no texture ownership when aborted before or during image decoding', async () => {
  images();
  const before = new AbortController();
  before.abort();
  expect(await createSceneMaterials(before.signal)).toBeUndefined();
  expect(evidence.decoded).toEqual([]);
  const during = new AbortController();
  vi.stubGlobal(
    'Image',
    class {
      src = '';
      async decode() {
        during.abort();
      }
    }
  );
  expect(await createSceneMaterials(during.signal)).toBeUndefined();
  expect(evidence.disposed).toEqual([]);
});
