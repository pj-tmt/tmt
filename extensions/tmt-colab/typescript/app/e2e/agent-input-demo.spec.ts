import { expect, test, type Page } from '@playwright/test';
import { capturePath } from './captures.js';

/** The design gallery's agent input frame: the real Chat composer, in every state it shows. */
const look = process.env.COLAB_LOOK ?? '1440:light';
const [width, theme] = look.split(':');

async function frame(page: Page, state: string, viewport = Number(width)) {
  await page.setViewportSize({ width: viewport, height: 800 });
  await page.emulateMedia({ colorScheme: theme === 'dark' ? 'dark' : 'light' });
  await page.route('**/src/main.tsx', (route) =>
    route.fulfill({
      contentType: 'text/javascript',
      body: `import { mount } from "/test/agent-input-demo.tsx"; mount(document.querySelector('#root'), new URLSearchParams(${JSON.stringify(`state=${state}`)}));`,
    }),
  );
  await page.goto('/');
}
const capture = async (page: Page, name: string) => {
  if (process.env.COLAB_CAPTURE_DIR) {
    await page.mouse.move(2, 2);
    await page.screenshot({
      path: capturePath(`agent-input-${name}-${look.replace(':', '-')}.png`),
    });
  }
};

const field = (page: Page) => page.getByRole('combobox', { name: 'Message' });

test('idle: the @astra chip is there and @ lists the agents, keys choose one', async ({ page }) => {
  await frame(page, 'idle');
  await expect(page.getByRole('button', { name: /^Remove mention: @astra/ })).toBeVisible();
  await capture(page, 'idle');
  await field(page).click();
  await page.keyboard.type(' @');
  const list = page.getByRole('listbox');
  await expect(list).toBeVisible();
  await expect(list.getByRole('option')).toHaveCount(3);
  await page.keyboard.press('ArrowDown');
  await capture(page, 'popup');
  await page.keyboard.press('Escape');
  await expect(list).toBeHidden();
  await capture(page, 'popup-closed');
});

test('draft: a long multi-line message stays in the input', async ({ page }) => {
  await frame(page, 'draft');
  await expect(page.getByText('Two things I need from you:')).toBeVisible();
  await capture(page, 'draft');
});

test('busy: a held write shows the message at once and leaves the input free', async ({ page }) => {
  await frame(page, 'busy');
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  const message = page.getByText('Is the Friday release still on track?');
  await expect(message).toBeVisible();
  await expect(field(page)).toBeEditable();
  await capture(page, 'busy');
});

test('refused: a refused send keeps the draft and names the reason', async ({ page }) => {
  await frame(page, 'refused');
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  await expect(page.getByRole('alert')).toBeVisible();
  await capture(page, 'refused');
});

test('attachments: files join the draft as chips, a refused one says so', async ({ page }) => {
  await frame(page, 'idle');
  const chooser = page.waitForEvent('filechooser');
  await page.getByRole('button', { name: 'Attach' }).click();
  await (
    await chooser
  ).setFiles([
    { name: 'release-notes.md', mimeType: 'text/markdown', buffer: Buffer.from('# Notes\n') },
    { name: 'refuse-this.txt', mimeType: 'text/plain', buffer: Buffer.from('too big') },
  ]);
  await expect(page.getByText('release-notes.md')).toBeVisible();
  await expect(page.getByText('refuse-this.txt')).toBeVisible();
  await page.waitForTimeout(900);
  await capture(page, 'attachments');
});

test('phone: the same input fills a 390 px screen', async ({ page }) => {
  await frame(page, 'idle', 390);
  const drawer = page.getByRole('dialog', { name: 'Chat' });
  await expect(drawer).toBeVisible();
  const box = (await drawer.boundingBox())!;
  expect(box.width).toBeGreaterThan(370);
  await field(page).click();
  await page.keyboard.type(' @');
  await expect(page.getByRole('listbox')).toBeVisible();
  await capture(page, 'phone-popup');
});
