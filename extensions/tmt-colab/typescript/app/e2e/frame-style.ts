import { expect, type Frame, type FrameLocator } from '@playwright/test';

/**
 * Read computed style from renderer frame content in one synchronous step in the frame.
 *
 * The renderer replaces its marks and overlays whenever it re-places them. A node a locator has
 * resolved can be detached by the time a function runs on it, and a detached node has no computed
 * style: every property reads empty ("" for a value that is really "none" or "2px"). So `read`
 * finds its node itself, in the frame's live document, and returns `undefined` until the node is
 * there and connected; the read repeats until it returns a value.
 */
export async function readFrameStyle<T>(
  frame: Frame | FrameLocator,
  read: (body: HTMLElement) => T | undefined,
): Promise<T> {
  let value: T | undefined;
  await expect
    .poll(
      async () => {
        value = await frame.locator('body').evaluate(read);
        return value !== undefined;
      },
      { message: 'the frame content has a computed style' },
    )
    .toBe(true);
  return value as T;
}
