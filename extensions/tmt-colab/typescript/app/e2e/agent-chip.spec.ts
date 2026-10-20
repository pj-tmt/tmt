import { expect, test, type Locator } from '@playwright/test';
import { capturePath } from './captures.js';

async function mountFixture(page: import('@playwright/test').Page) {
  await page.goto('/');
  await page.evaluate(async () => {
    const path = '/test/agent-chip-browser.tsx';
    (await import(path)).mount();
  });
  return page.locator('#agent-chip-fixture');
}

/** The computed colour of a CSS colour expression, as the page resolves it. */
const resolved = (host: Locator, colour: string) =>
  host.evaluate((node, colour) => {
    const probe = document.createElement('span');
    probe.style.color = colour;
    node.append(probe);
    const value = getComputedStyle(probe).color;
    probe.remove();
    return value;
  }, colour);
const dot = (mark: Locator) =>
  mark.evaluate((node) => {
    const { width, height, top, right, backgroundColor } = getComputedStyle(node, '::after');
    return { width, height, top, right, backgroundColor };
  });

test('a working agent keeps its robot and gains a static dot; nothing animates', async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  const host = await mountFixture(page);
  const idle = host.locator('[data-row=inline-idle] .agent-chip');
  const working = host.locator('[data-row=inline-working] .agent-chip');
  await expect(idle.locator('.agent-chip-icon')).toBeVisible();
  await expect(working.locator('.agent-chip-icon')).toBeVisible();
  await expect(working.locator('.agent-chip-name')).toHaveText('Atlas');
  await expect(idle.locator('[data-working]')).toHaveCount(0);
  await expect(working.locator('[data-working]')).toHaveCount(1);

  // The dot is 6px, at the icon's top-right inside its box, in the working colour.
  const mark = working.locator('.agent-chip-mark');
  expect(await dot(mark)).toEqual({
    width: '6px',
    height: '6px',
    top: '0px',
    right: '0px',
    backgroundColor: await resolved(host, 'var(--tmt-ui-color-working)'),
  });
  expect((await dot(idle.locator('.agent-chip-mark'))).width).not.toBe('6px');
  // The robot and the text stay neutral.
  expect(await working.locator('.agent-chip-icon').evaluate((n) => getComputedStyle(n).color)).toBe(
    await idle.locator('.agent-chip-icon').evaluate((n) => getComputedStyle(n).color),
  );

  // Words for assistive technology and the hover text; the chip's width does not change.
  await expect(working).toHaveAttribute('title', 'Atlas, working');
  await expect(host.locator('[data-row=inline-working]')).toMatchAriaSnapshot(
    '- paragraph: /Waiting for Atlas ?, working/',
  );
  await expect(host.locator('[data-row=inline-idle]')).toMatchAriaSnapshot(
    '- paragraph: Waiting for Atlas',
  );
  expect((await working.boundingBox())!.width).toBeCloseTo((await idle.boundingBox())!.width, 0);

  // The bordered form takes the same 16px mark box, with the dot in it.
  const chip = host.locator('[data-row=chip-working] .agent-chip-mark');
  expect(await chip.evaluate((node: HTMLElement) => [node.offsetWidth, node.offsetHeight])).toEqual(
    [16, 16],
  );
  expect((await dot(chip)).width).toBe('6px');
  // Not even with motion allowed: no chip carries an animation.
  expect(await page.evaluate(() => document.getAnimations().length)).toBe(0);
});

test('the icon keeps a visible gap before the name in every form and state', async ({ page }) => {
  const host = await mountFixture(page);
  for (const row of ['inline-idle', 'inline-working', 'chip', 'chip-working']) {
    const chip = host.locator(`[data-row=${row}] .agent-chip`);
    const icon = (await chip.locator('.agent-chip-icon').boundingBox())!;
    const name = (await chip.locator('.agent-chip-name').boundingBox())!;
    // The 4px between the mark box and the name, whatever the state or the global icon size.
    expect(name.x - (icon.x + icon.width), row).toBeGreaterThanOrEqual(3.5);
    // The icon fills its mark box: it neither overflows into the gap nor shrinks away from it.
    const mark = (await chip.locator('.agent-chip-mark').boundingBox())!;
    expect(icon.x + icon.width, row).toBeCloseTo(mark.x + mark.width, 1);
  }
});

test('forced colours draw the dot in the system text colour', async ({ page }) => {
  await page.emulateMedia({ forcedColors: 'active' });
  const host = await mountFixture(page);
  const mark = host.locator('[data-row=inline-working] .agent-chip-mark');
  await expect(mark).toBeVisible();
  expect((await dot(mark)).backgroundColor).toBe(await resolved(host, 'CanvasText'));
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
