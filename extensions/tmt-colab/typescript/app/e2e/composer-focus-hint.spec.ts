import { expect, test, type Page } from '@playwright/test';
import { text } from '../src/strings.js';
import { destination, id } from '../test/ask-fixtures.js';
import { capturePath } from './captures.js';

/** The Chat composer's agent list and failed-send hint, at desktop and phone width (#4478). */
const fixture = '/test/annotation-browser.tsx';
const run = (page: Page, method: string, argument?: unknown) =>
  page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );

const capture = async (page: Page, name: string, width: number) => {
  if (process.env.COLAB_CAPTURE_DIR) {
    await page.mouse.move(2, 2);
    await page.screenshot({ path: capturePath(`composer-${name}-${width}.png`) });
  }
};

async function mount(page: Page, width: number, mode = 'accepted') {
  await page.setViewportSize({ width, height: 844 });
  await page.goto('/');
  await run(page, 'setSurface', 'chat');
  await run(page, 'setAgents', [
    { ...destination(), agentName: 'alpha', presence: 'active' },
    { ...destination(), agent: id(7), agentName: 'beta', presence: 'active' },
  ]);
  await run(page, 'setCreator', false);
  await run(page, 'setDirectory', 'ok');
  await run(page, 'setEchoHost', true);
  await run(page, 'mount', mode);
  const input = page.getByRole('combobox', { name: 'Message', exact: true });
  await expect(input).toBeVisible();
  await expect(page.locator('.annotation-hint[role="status"]')).not.toContainText('Checking');
  return input;
}

for (const width of [1440, 390]) {
  test(`the keyboard-active agent gets the one inset focus frame: ${width}px`, async ({ page }) => {
    const input = await mount(page, width);
    await input.pressSequentially('@');
    const list = page.getByRole('listbox');
    await expect(list.getByRole('option')).toHaveCount(2);
    await input.press('ArrowDown');
    const active = list.locator('[role=option][data-active="true"]');
    await expect(active).toHaveCount(1);
    const frame = await active.evaluate((option) => {
      const probe = document.createElement('i');
      probe.style.outline = 'var(--tmt-ui-focus-width) solid var(--tmt-ui-color-focus)';
      document.body.append(probe);
      const token = getComputedStyle(probe);
      const expected = { width: token.outlineWidth, color: token.outlineColor };
      probe.remove();
      const style = getComputedStyle(option);
      const box = option.getBoundingClientRect();
      const parent = option.parentElement!.getBoundingClientRect();
      return {
        expected,
        style: style.outlineStyle,
        width: style.outlineWidth,
        color: style.outlineColor,
        offset: parseFloat(style.outlineOffset),
        inside: box.left >= parent.left && box.right <= parent.right,
      };
    });
    await capture(page, 'agent-list', width);
    expect(frame.style).toBe('solid');
    expect(frame.width).toBe(frame.expected.width);
    expect(frame.color).toBe(frame.expected.color);
    // Inset: the frame is drawn inside the option, so the list never clips it.
    expect(frame.offset).toBeLessThan(0);
    expect(frame.inside).toBe(true);
    // An option that is not active has no frame.
    const other = list.locator('[role=option][data-active="false"]').first();
    expect(await other.evaluate((option) => getComputedStyle(option).outlineStyle)).toBe('none');
  });

  test(`the failed-send hint sits on its own line below the send row: ${width}px`, async ({
    page,
  }) => {
    const input = await mount(page, width, 'write-failure');
    await input.pressSequentially('Kept draft.');
    const send = page.getByRole('button', { name: 'Send', exact: true });
    const before = (await send.boundingBox())!;
    await input.press('Enter');
    const hint = page.getByText(text.messageUnsentFirst);
    await expect(hint).toBeVisible();
    const sendBox = (await send.boundingBox())!;
    const hintBox = (await hint.boundingBox())!;
    expect(hintBox.y).toBeGreaterThanOrEqual(sendBox.y + sendBox.height - 1);
    // The send button keeps its place on the right of its own row.
    expect(Math.abs(sendBox.x + sendBox.width - (before.x + before.width))).toBeLessThanOrEqual(1);
    await capture(page, 'failed-hint', width);
    // The hint is one line, not wrapped.
    expect(hintBox.height).toBeLessThan(24);
  });
}
