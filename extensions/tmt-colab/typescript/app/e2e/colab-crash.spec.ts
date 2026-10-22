import { expect, test, type Page } from '@playwright/test';
import { capturePath } from './captures.js';

/**
 * A crash inside Colab, as a route of the TMT web app, keeps the shell (#4373): the host's rail and
 * header slot stay, Colab's error view fills the slot, and leaving the route and coming back builds
 * a clean root. Run over the local preview (test/spa-links-browser.tsx).
 */
const fixture = '/test/spa-links-browser.tsx';

/** The look a capture run uses, `COLAB_LOOK=390:dark`; the assertions hold in every look. */
const look = process.env.COLAB_LOOK ?? '1440:light';
const capture = async (page: Page, name: string) => {
  if (process.env.COLAB_CAPTURE_DIR)
    await page.screenshot({ path: capturePath(`crash-${name}-${look.replace(':', '-')}.png`) });
};

async function start(page: Page) {
  const [width, scheme] = look.split(':');
  await page.setViewportSize({ width: Number(width), height: 900 });
  await page.emulateMedia({ colorScheme: scheme === 'dark' ? 'dark' : 'light' });
  await page.goto('/');
}
const call = <T>(page: Page, name: string, ...args: unknown[]) =>
  page.evaluate(
    async ({ fixture, name, args }) =>
      ((await import(fixture)) as Record<string, (...args: unknown[]) => unknown>)[name]!(...args),
    { fixture, name, args },
  ) as Promise<T>;
const slot = (page: Page) =>
  call<{
    bar: string;
    barHeaders: number;
    main: string;
    mainHeaders: number;
    alert: string;
  }>(page, 'slotState');

test('a route that throws keeps Colab’s header in the host’s header slot', async ({ page }) => {
  await start(page);
  await call(page, 'mountModule', '/colab/', true);
  await expect(page.getByRole('alert')).toContainText('Page unavailable');
  // The header is in the host's bar, once, and not inside the content area.
  expect(await slot(page)).toMatchObject({ barHeaders: 1, mainHeaders: 0 });
  expect((await slot(page)).bar).toContain('Page unavailable');
  await capture(page, 'route');
});

test('leaving a crashed route and coming back mounts a clean root', async ({ page }) => {
  await start(page);
  await call(page, 'mountModule', '/colab/', true);
  await expect(page.getByRole('alert')).toBeVisible();
  await call(page, 'unmountModule');
  await call(page, 'mountModule', '/colab/');
  await expect(page.getByRole('alert')).toHaveCount(0);
  await expect.poll(async () => (await slot(page)).barHeaders).toBe(1);
});

test('a crash above the router shows Colab’s error in the slot and Try again recovers', async ({
  page,
}) => {
  await start(page);
  await call(page, 'mountBoundary');
  await expect(page.getByRole('alert')).toContainText('Colab stopped');
  expect(await slot(page)).toMatchObject({ barHeaders: 1, mainHeaders: 0 });
  await capture(page, 'boundary');
  await call(page, 'healBoundary');
  await page.getByRole('button', { name: 'Try again' }).click();
  await expect(page.getByText('Recovered')).toBeVisible();
  await expect(page.getByRole('alert')).toHaveCount(0);
});

test('Try again that fails again stays on the error view', async ({ page }) => {
  await start(page);
  await call(page, 'mountBoundary');
  await page.getByRole('button', { name: 'Try again' }).click();
  await expect(page.getByRole('alert')).toContainText('Colab stopped');
  await call(page, 'healBoundary');
  await page.getByRole('button', { name: 'Try again' }).click();
  await expect(page.getByText('Recovered')).toBeVisible();
});
