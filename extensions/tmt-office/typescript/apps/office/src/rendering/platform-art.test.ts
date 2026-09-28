import { afterEach, expect, it, vi } from 'vitest';
import { createPlatformArt, PLATFORM_FRAMES } from './platform-art.js';

const evidence = vi.hoisted(() => ({ disposed: 0, decoded: 0 }));
vi.mock('pixi.js', () => ({
  CanvasSource: class {},
  Texture: class {
    destroy() {
      evidence.disposed++;
    }
  },
}));
afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  evidence.disposed = evidence.decoded = 0;
});

function images(width = 1536) {
  vi.stubGlobal(
    'Image',
    class {
      naturalWidth = width;
      naturalHeight = 1024;
      async decode() {
        evidence.decoded++;
      }
    }
  );
}

it('decodes once, clips silhouettes once, and disposes textures once', async () => {
  images();
  const context = {
    beginPath: vi.fn(),
    moveTo: vi.fn(),
    lineTo: vi.fn(),
    closePath: vi.fn(),
    clip: vi.fn(),
    drawImage: vi.fn(),
    fillRect: vi.fn(),
    getImageData: vi.fn(() => ({
      data: new Uint8ClampedArray([200, 240, 230, 255, 20, 45, 55, 255]),
    })),
    putImageData: vi.fn(),
  };
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(
    context as unknown as ReturnType<HTMLCanvasElement['getContext']>
  );
  const art = (await createPlatformArt(new AbortController().signal))!;
  const count = Object.keys(PLATFORM_FRAMES).length;
  expect(evidence.decoded).toBe(1);
  expect(context.drawImage).toHaveBeenCalledTimes(count);
  // Four alloy panels, paired rivets, a service grille and restrained wear.
  expect(context.fillRect).toHaveBeenCalledTimes(36);
  // Both lamp uses share the housing; only the bright inset pixels change.
  expect(context.clip).toHaveBeenCalledTimes(6);
  expect(context.getImageData).toHaveBeenCalledTimes(2);
  expect(context.getImageData).toHaveBeenCalledWith(39, 51, 81, 25);
  expect(context.getImageData).toHaveBeenCalledWith(57, 51, 81, 25);
  expect(context.putImageData).toHaveBeenCalledWith(
    { data: new Uint8ClampedArray([211, 144, 240, 255, 20, 45, 55, 255]) },
    39,
    51
  );
  expect(context.putImageData).toHaveBeenCalledWith(
    { data: new Uint8ClampedArray([211, 144, 240, 255, 20, 45, 55, 255]) },
    57,
    51
  );
  expect(Object.keys(art.textures)).toHaveLength(count);
  art.dispose();
  art.dispose();
  expect(evidence.disposed).toBe(count);
});

it('rejects unreviewed source dimensions before creating textures', async () => {
  images(1024);
  await expect(createPlatformArt(new AbortController().signal)).rejects.toThrow('reviewed frames');
  expect(evidence.disposed).toBe(0);
});

it('allocates nothing for an aborted mount', async () => {
  images();
  const controller = new AbortController();
  controller.abort();
  expect(await createPlatformArt(controller.signal)).toBeUndefined();
  expect(evidence.decoded).toBe(0);
});
