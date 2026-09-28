import { Application } from 'pixi.js';
// Despite its name, this official polyfill replaces dynamic code generation.
import 'pixi.js/unsafe-eval';
import { createSceneFrames } from './scene-frames.js';

/** GPU and browser resources belong to this mount, not a global app or ticker. */
export async function createSceneApplication(host: HTMLElement, signal: AbortSignal) {
  if (signal.aborted) return undefined;
  const application = new Application();
  try {
    await application.init({
      width: Math.max(1, host.clientWidth),
      height: Math.max(1, host.clientHeight),
      resolution: Math.min(window.devicePixelRatio || 1, 2),
      autoDensity: true,
      antialias: false,
      autoStart: false,
      sharedTicker: false,
      preference: 'webgl',
      powerPreference: 'low-power',
      background: '#263d33',
    });
  } catch (error) {
    // A failed renderer initialization has not necessarily installed app plugins.
    if (application.renderer) application.destroy(true, { children: true });
    else application.stage.destroy({ children: true });
    if (signal.aborted) return undefined;
    throw error;
  }
  if (signal.aborted) {
    application.destroy(true, { children: true });
    return undefined;
  }

  const frames = createSceneFrames(() => application.render());
  host.appendChild(application.canvas);
  let disposed = false;
  const resize = new ResizeObserver(() => {
    if (disposed) return;
    const width = Math.max(1, host.clientWidth);
    const height = Math.max(1, host.clientHeight);
    if (application.screen.width === width && application.screen.height === height) return;
    application.renderer.resize(width, height);
    frames.invalidate();
  });
  resize.observe(host);
  function dispose() {
    if (disposed) return;
    disposed = true;
    signal.removeEventListener('abort', dispose);
    resize.disconnect();
    frames.dispose();
    application.destroy(true, { children: true });
  }
  signal.addEventListener('abort', dispose, { once: true });
  frames.invalidate();
  return {
    application,
    invalidate: frames.invalidate,
    dispose,
  };
}
