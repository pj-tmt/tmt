import { expect, test } from '@playwright/test';
import { capturePath } from './captures.js';

async function mountFixture(page: import('@playwright/test').Page) {
  await page.goto('/');
  await page.evaluate(async () => {
    const path = '/test/agent-chip-browser.tsx';
    (await import(path)).mount();
  });
  return page.locator('#agent-chip-fixture');
}

test('the icon turns into a labelled spinner only while the agent works', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  const host = await mountFixture(page);
  const idle = host.locator('[data-row=inline-idle] .agent-chip');
  const working = host.locator('[data-row=inline-working] .agent-chip');
  await expect(idle.locator('.agent-chip-icon')).toBeVisible();
  await expect(idle.getByRole('img')).toHaveCount(0);
  await expect(working.locator('.agent-chip-icon')).toHaveCount(0);
  await expect(working.getByRole('img', { name: 'working' })).toBeVisible();
  await expect(working.locator('.agent-chip-name')).toHaveText('Atlas');
  await expect(working.locator('.agent-chip-spinner')).toHaveCSS(
    'animation-name',
    'agent-chip-spin',
  );
  // The bordered form works too, and the spinner takes the same 16px mark box.
  const chip = host.locator('[data-row=chip-working] .agent-chip-spinner');
  // Layout size: the turning ring's bounding box changes with the frame.
  expect(await chip.evaluate((node: HTMLElement) => [node.offsetWidth, node.offsetHeight])).toEqual(
    [16, 16],
  );
});

test('reduced motion keeps the spinner ring still', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  const host = await mountFixture(page);
  const spinner = host.locator('[data-row=inline-working] .agent-chip-spinner');
  await expect(spinner).toBeVisible();
  await expect(spinner).toHaveCSS('animation-name', 'none');
});

test('a sentence names its agents as inline chips and a list separates them', async ({ page }) => {
  const host = await mountFixture(page);
  const sentence = host.locator('[data-row=sentence]');
  await expect(sentence).toHaveText('No reply yet from Atlas');
  await expect(sentence.locator('.agent-chip')).toHaveCount(1);
  const list = host.locator('[data-row=list]');
  await expect(list).toHaveText('Not notified: alpha, beta');
  await expect(list.locator('.agent-chip')).toHaveCount(2);
  const removable = host.locator('[data-row=chip] .agent-chip');
  await expect(removable.locator('.agent-chip-machine')).toHaveText('· My machine');
  await expect(removable.getByRole('button', { name: 'Remove Atlas' })).toBeVisible();
});

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const)
    test(`3712 agent chip forms, ${width} ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 700 });
      await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
      await page.addInitScript((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      const host = await mountFixture(page);
      await expect(host.locator('.agent-chip')).toHaveCount(8);
      await host.screenshot({ path: capturePath(`3712-agent-chip-${width}-${theme}.png`) });
    });
