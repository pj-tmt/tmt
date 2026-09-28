import { expect } from '@playwright/test';
import { writeFile } from 'node:fs/promises';
import { cpus, platform, arch } from 'node:os';
import type { Page, TestInfo } from '@playwright/test';

export interface SceneActivity {
  draws: number;
  submittedVertices: number;
  texturesCreated: number;
  texturesDeleted: number;
  texturesLive: number;
}

/** Test-only observation of actual WebGL submissions/resources, not a product debug API. */
export async function installDrawObserver(page: Page) {
  // Install the stable media mode before the scene is created so the first frame
  // and every later comparison exclude the intentional 20 Hz bridge glow.
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.addInitScript(() => {
    const activity = {
      draws: 0,
      submittedVertices: 0,
      texturesCreated: 0,
      texturesDeleted: 0,
      texturesLive: 0,
    };
    const textures = new WeakSet<WebGLTexture>();
    Object.defineProperty(window, '__officeDrawCount', { get: () => activity.draws });
    Object.defineProperty(window, '__officeSceneActivity', { get: () => ({ ...activity }) });
    for (const type of [WebGLRenderingContext, WebGL2RenderingContext]) {
      for (const method of [
        'drawArrays',
        'drawElements',
        'drawArraysInstanced',
        'drawElementsInstanced',
        'drawRangeElements',
      ]) {
        const original = Reflect.get(type.prototype, method);
        if (typeof original !== 'function') continue;
        Object.defineProperty(type.prototype, method, {
          configurable: true,
          writable: true,
          value: function (this: WebGLRenderingContext, ...args: number[]) {
            activity.draws++;
            activity.submittedVertices +=
              method === 'drawArrays'
                ? args[2]!
                : method === 'drawArraysInstanced'
                  ? args[2]! * args[3]!
                  : method === 'drawElementsInstanced'
                    ? args[1]! * args[4]!
                    : method === 'drawRangeElements'
                      ? args[3]!
                      : args[1]!;
            return Reflect.apply(original, this, args);
          },
        });
      }
      const create = type.prototype.createTexture;
      const destroy = type.prototype.deleteTexture;
      type.prototype.createTexture = function () {
        const texture = Reflect.apply(create, this, []) as ReturnType<typeof create>;
        if (texture) {
          textures.add(texture);
          activity.texturesCreated++;
          activity.texturesLive++;
        }
        return texture;
      };
      type.prototype.deleteTexture = function (texture: WebGLTexture | null) {
        Reflect.apply(destroy, this, [texture]);
        if (texture && textures.delete(texture)) {
          activity.texturesDeleted++;
          activity.texturesLive--;
        }
      };
    }
  });
}

/** Finish scheduled paints before reading cumulative counters; no wall-clock sleep. */
export async function sceneActivity(page: Page): Promise<SceneActivity> {
  return page.evaluate(async () => {
    await new Promise(requestAnimationFrame);
    await new Promise(requestAnimationFrame);
    return Reflect.get(window, '__officeSceneActivity');
  });
}

/** Exact world comparison excludes changing DOM controls and compositor shadows. */
export async function captureWorldScene(page: Page, info: TestInfo, name: string) {
  await sceneActivity(page);
  const hud = page.locator('.world-hud-layout');
  // Native CSP rejects injected stylesheets. Change only existing presentation;
  // never disable CSP, resize the canvas or mutate application state for a capture.
  const visibility = await hud.evaluate((element: HTMLElement) => {
    const previous = element.style.visibility;
    element.style.visibility = 'hidden';
    return previous;
  });
  try {
    await expect(hud).toHaveCSS('visibility', 'hidden');
    return await page.locator('.office-canvas canvas').screenshot({ path: info.outputPath(name) });
  } finally {
    await hud.evaluate((element: HTMLElement, value) => {
      element.style.visibility = value;
    }, visibility);
  }
}

export async function observeIdleScene(page: Page, testInfo: TestInfo, name: string) {
  await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
  const session = await page.context().newCDPSession(page);
  await session.send('Performance.enable');
  // Finish the scheduled initial paint before BOTH the CPU baseline and draw
  // baseline. Taking CPU metrics first mislabels startup work as idle work.
  await page.evaluate(async () => {
    await new Promise(requestAnimationFrame);
    await new Promise(requestAnimationFrame);
  });
  const startMetrics = await session.send('Performance.getMetrics');
  const observation = await page.evaluate(async () => {
    const before = Number(Reflect.get(window, '__officeDrawCount'));
    const start = performance.now();
    await new Promise((resolve) => setTimeout(resolve, 400));
    return {
      before,
      after: Number(Reflect.get(window, '__officeDrawCount')),
      durationMs: performance.now() - start,
      viewport: { width: innerWidth, height: innerHeight, dpr: devicePixelRatio },
      userAgent: navigator.userAgent,
      hardwareConcurrency: navigator.hardwareConcurrency,
    };
  });
  const endMetrics = await session.send('Performance.getMetrics');
  await session.detach();
  const metric = (metrics: { name: string; value: number }[], name: string) =>
    metrics.find((item) => item.name === name)?.value;
  const taskBefore = metric(startMetrics.metrics, 'TaskDuration');
  const taskAfter = metric(endMetrics.metrics, 'TaskDuration');
  const evidence = {
    ...observation,
    host: { platform: platform(), architecture: arch(), cpu: cpus()[0]?.model },
    taskDurationSeconds:
      taskBefore === undefined || taskAfter === undefined ? null : taskAfter - taskBefore,
    jsHeapUsedBytes: metric(endMetrics.metrics, 'JSHeapUsedSize'),
    note: 'Headless Chromium renderer-thread observation; heap is not total browser or GPU memory.',
  };
  const path = testInfo.outputPath(`${name}-idle-rendering.json`);
  await writeFile(path, JSON.stringify(evidence, null, 2));
  await testInfo.attach(`${name}-idle-rendering`, {
    contentType: 'application/json',
    path,
  });
  expect(observation.before).toBeGreaterThan(0);
  expect(observation.after).toBe(observation.before);
}
