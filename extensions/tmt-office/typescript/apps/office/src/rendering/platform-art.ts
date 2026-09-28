import { CanvasSource, Texture } from 'pixi.js';
import sourceUrl from './assets/mechanical-platform-v1.png';

/** Source-pixel contours exclude the study sheet's opaque background. These are
 * authored sprite silhouettes, not color-key guesses or per-frame GPU masks. */
const LAMP_FRAME = {
  x: 439,
  y: 565,
  width: 153,
  height: 111,
  outline: [0, 0, 151, 0, 153, 69, 128, 96, 99, 96, 96, 110, 74, 110, 66, 97, 23, 96, 0, 75],
} as const;
const CORNER_FRAME = {
  x: 797,
  y: 559,
  width: 143,
  height: 127,
  outline: [
    13, 0, 42, 8, 142, 8, 142, 98, 115, 99, 106, 122, 86, 122, 75, 99, 40, 91, 29, 55, 0, 27, 0, 9,
  ],
} as const;
export const PLATFORM_FRAMES = {
  rim: { x: 115, y: 66, width: 65, height: 12 },
  face: { x: 172, y: 582, width: 102, height: 61 },
  lamp: LAMP_FRAME,
  meetingLamp: LAMP_FRAME,
  bracket: {
    x: 633,
    y: 557,
    width: 122,
    height: 135,
    outline: [
      18, 0, 105, 0, 122, 16, 115, 41, 108, 44, 108, 106, 82, 110, 80, 134, 51, 134, 43, 111, 20,
      106, 14, 40, 0, 27, 0, 14,
    ],
  },
  corner: CORNER_FRAME,
  meetingCorner: CORNER_FRAME,
  // Interior only: repeating the authored side seams creates an off-center rail.
  deck: { x: 857, y: 780, width: 46, height: 128 },
  bridgeRail: { x: 823, y: 780, width: 20, height: 120 },
  bridgeDock: {
    x: 1344,
    y: 820,
    width: 150,
    height: 66,
    outline: [
      2, 10, 14, 0, 27, 8, 126, 8, 135, 0, 148, 6, 150, 58, 138, 66, 125, 58, 27, 58, 18, 66, 0, 60,
    ],
  },
} as const;
export type PlatformTextures = Record<keyof typeof PLATFORM_FRAMES, Texture>;

/** One decoded source, fixed nearest-neighbor sprite textures for every room. */
export async function createPlatformArt(signal: AbortSignal) {
  if (signal.aborted) return;
  const image = new Image();
  image.src = sourceUrl;
  await image.decode();
  if (signal.aborted) return;
  if (image.naturalWidth !== 1536 || image.naturalHeight !== 1024)
    throw new Error('The mechanical platform sheet does not match its reviewed frames.');
  const textures = {} as PlatformTextures;
  function dispose() {
    for (const key of Object.keys(textures) as (keyof PlatformTextures)[]) {
      textures[key].destroy(true);
      delete textures[key];
    }
  }
  try {
    for (const key of Object.keys(PLATFORM_FRAMES) as (keyof PlatformTextures)[]) {
      const frame = PLATFORM_FRAMES[key];
      const canvas = document.createElement('canvas');
      canvas.width = frame.width;
      canvas.height = frame.height;
      const context = canvas.getContext('2d');
      if (!context) throw new Error('Mechanical platform artwork could not initialize.');
      if ('outline' in frame) {
        context.beginPath();
        const points = frame.outline;
        context.moveTo(points[0], points[1]);
        for (let i = 2; i < points.length; i += 2) context.lineTo(points[i]!, points[i + 1]!);
        context.closePath();
        context.clip();
      }
      context.drawImage(
        image,
        frame.x,
        frame.y,
        frame.width,
        frame.height,
        0,
        0,
        frame.width,
        frame.height
      );
      if (key === 'meetingLamp' || key === 'meetingCorner') {
        // Only the authored light inset changes hue. The mechanical housing,
        // silhouette and pixel luminance are shared with ordinary platforms.
        const insetX = key === 'meetingLamp' ? 39 : 57;
        const pixels = context.getImageData(insetX, 51, 81, 25);
        for (let i = 0; i < pixels.data.length; i += 4) {
          const r = pixels.data[i]!,
            g = pixels.data[i + 1]!,
            b = pixels.data[i + 2]!;
          if (g >= 130 && g >= r && b >= r) {
            pixels.data[i] = Math.round(g * 0.88);
            pixels.data[i + 1] = Math.round(g * 0.6);
            pixels.data[i + 2] = g;
          }
        }
        context.putImageData(pixels, insetX, 51);
      }
      if (key === 'deck') {
        // Bake a deterministic alloy/service-panel mix once into the shared
        // texture. No per-floor objects, random noise or extra runtime filters.
        for (let panel = 0; panel < 4; panel++) {
          const y = panel * 32;
          context.fillStyle = panel % 2 ? '#8e927b20' : '#244b5928';
          context.fillRect(0, y, frame.width, 32);
          context.fillStyle = '#152e3680';
          context.fillRect(3, y + 2, frame.width - 6, 1);
          context.fillStyle = '#b5bc9570';
          context.fillRect(3, y + 3, frame.width - 6, 1);
          for (const x of [4, frame.width - 6]) {
            context.fillStyle = '#18313bcc';
            context.fillRect(x, y + 7, 3, 3);
            context.fillStyle = '#b5b99c';
            context.fillRect(x, y + 7, 2, 1);
          }
          if (panel === 2) {
            context.fillStyle = '#142b3666';
            for (let line = 0; line < 4; line++)
              context.fillRect(14, y + 10 + line * 3, frame.width - 28, 1);
          }
          context.fillStyle = '#d6cfac35';
          context.fillRect(12 + panel * 2, y + 24, 8, 1);
        }
      }
      textures[key] = new Texture({
        source: new CanvasSource({ resource: canvas, scaleMode: 'nearest' }),
      });
    }
    return { textures, dispose };
  } catch (error) {
    dispose();
    throw error;
  }
}
