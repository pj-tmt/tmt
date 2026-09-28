import { afterEach, expect, it, vi } from 'vitest';
import { createSceneTextures } from './scene-textures.js';
import detailed from '../../../../../contracts/avatar-pack-v2-sample.tmtavatar.json';
import { avatarRaster, decodeAvatarPack } from '../avatars/avatar-contract.js';

const gpu = vi.hoisted(() => ({ allocations: vi.fn(), releases: vi.fn() }));
vi.mock('pixi.js', () => ({
  CanvasSource: class {
    constructor(options: unknown) {
      gpu.allocations(options);
    }
  },
  Texture: class {
    destroy = vi.fn((releaseSource: boolean) => gpu.releases(releaseSource));
  },
}));
afterEach(() => {
  vi.restoreAllMocks();
  vi.clearAllMocks();
});

const art = { pixels: ['01', '10'], palette: ['#00000000', '#abcdefff'] };

it('keeps avatar v2 encoding intact when allocating the shared GPU texture', () => {
  const cells: Array<[number, number, number, number, string]> = [];
  const context = {
    fillStyle: '',
    fillRect(x: number, y: number, width: number, height: number) {
      cells.push([x, y, width, height, this.fillStyle]);
    },
  };
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation(((kind: string) => {
    expect(kind).toBe('2d');
    return context;
  }) as unknown as HTMLCanvasElement['getContext']);
  const pack = decodeAvatarPack(detailed);
  const textures = createSceneTextures();
  textures.get('avatar-v2', avatarRaster(pack, pack.avatars[0]!));
  expect(cells).toContainEqual([12, 16, 2, 1, '#b3f6d9ff']);
  expect(gpu.allocations.mock.calls[0]![0].resource.width).toBe(32);
  expect(gpu.allocations.mock.calls[0]![0].resource.height).toBe(48);
  textures.dispose();
});

it('clips trusted display text to its admitted rectangle and shares the existing texture lifetime', () => {
  const context = {
    fillRect: vi.fn(),
    save: vi.fn(),
    beginPath: vi.fn(),
    rect: vi.fn(),
    clip: vi.fn(),
    fillText: vi.fn(),
    restore: vi.fn(),
    font: '',
    fillStyle: '',
    textAlign: '',
    textBaseline: '',
  };
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation(((kind: string) => {
    expect(kind).toBe('2d');
    return context;
  }) as unknown as HTMLCanvasElement['getContext']);
  const textures = createSceneTextures();
  textures.begin();
  const textArt = {
    ...art,
    text: { value: 'Hello', color: '#fffbed', x: 0, y: 0, width: 2, height: 1 },
  };
  const texture = textures.get('prop/with-text', textArt);
  expect(context.rect).toHaveBeenCalledExactlyOnceWith(0, 0, 2, 1);
  expect(context.clip).toHaveBeenCalledTimes(1);
  expect(context.fillText).toHaveBeenCalledExactlyOnceWith('Hello', 1, 0.5);
  expect(context.textAlign).toBe('center');
  expect(context.textBaseline).toBe('middle');
  expect(context.font.endsWith('px monospace')).toBe(true);
  expect(context.restore).toHaveBeenCalledTimes(1);
  expect(textures.get('prop/with-text', textArt)).toBe(texture);
  expect(context.fillText).toHaveBeenCalledTimes(1);
  textures.end();
  textures.begin();
  textures.end();
  expect(texture.destroy).toHaveBeenCalledExactlyOnceWith(true);
});

it('shares admitted artwork across objects and snapshots, releasing only unused textures', () => {
  const fillRect = vi.fn();
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation(((kind: string) => {
    expect(kind).toBe('2d');
    return { fillRect };
  }) as unknown as HTMLCanvasElement['getContext']);
  const textures = createSceneTextures();
  textures.begin();
  const first = textures.get('pack/first', art);
  expect(textures.get('pack/first', art)).toBe(first);
  const second = textures.get('pack/second', art);
  textures.end();
  expect(gpu.allocations).toHaveBeenCalledTimes(2);
  expect(gpu.allocations).toHaveBeenCalledWith(expect.objectContaining({ scaleMode: 'nearest' }));
  expect(fillRect.mock.calls).toEqual([
    [1, 0, 1, 1],
    [0, 1, 1, 1],
    [1, 0, 1, 1],
    [0, 1, 1, 1],
  ]);
  textures.begin();
  expect(textures.get('pack/second', art)).toBe(second);
  textures.end();
  expect(first.destroy).toHaveBeenCalledWith(true);
  expect(second.destroy).not.toHaveBeenCalled();
  textures.dispose();
  textures.dispose();
  expect(first.destroy).toHaveBeenCalledTimes(1);
  expect(second.destroy).toHaveBeenCalledExactlyOnceWith(true);
});

it('does not cache a failed raster allocation', () => {
  const context = vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(null);
  const textures = createSceneTextures();
  textures.begin();
  expect(() => textures.get('retry', art)).toThrow('Pixel artwork could not initialize.');
  expect(gpu.allocations).not.toHaveBeenCalled();
  context.mockImplementation(((kind: string) => {
    expect(kind).toBe('2d');
    return { fillRect: vi.fn() };
  }) as unknown as HTMLCanvasElement['getContext']);
  textures.get('retry', art);
  expect(gpu.allocations).toHaveBeenCalledTimes(1);
  textures.dispose();
  expect(gpu.releases).toHaveBeenCalledExactlyOnceWith(true);
});
