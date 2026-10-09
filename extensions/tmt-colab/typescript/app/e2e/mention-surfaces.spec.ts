import { pageAction } from '../test/page-actions.js';
import { expect, test, type Locator, type Page } from '@playwright/test';
import { mkdirSync, writeFileSync } from 'node:fs';
import { captureDirectory } from './captures.js';
const fixture = '/test/ask-page-browser.tsx';
async function run(page: Page, method: string, argument?: unknown) {
  return page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
}
async function open(page: Page, surface: string, checking: boolean) {
  await page.goto('/');
  await run(page, 'mount', { creator: true, checking });
  const app = page.locator('#ask-page-fixture');
  if (surface === 'thread') {
    await run(page, 'conversation', { surface: 'thread', state: 'replied' });
    await page.frameLocator('#ask-page-fixture iframe').locator('[data-colab-thread]').click();
  } else if (surface === 'chat') {
    const toggle = await pageAction(app, 'Chat');
    await toggle.click();
  } else {
    await page
      .frameLocator('#ask-page-fixture iframe')
      .locator('#selected')
      .evaluate((node) => {
        const range = node.ownerDocument.createRange();
        range.selectNodeContents(node);
        const selection = node.ownerDocument.getSelection()!;
        selection.removeAllRanges();
        selection.addRange(range);
      });
    await page.getByTestId('selection-ask').click();
  }
  return page.getByTestId(
    surface === 'chat'
      ? 'chat-panel'
      : surface === 'thread'
        ? 'comment-thread'
        : 'annotation-window',
  );
}
async function appearance(composer: Locator) {
  return composer.evaluate((node) => {
    const read = (element: Element) => {
      const style = getComputedStyle(element);
      return Object.fromEntries(
        [
          'border-radius',
          'box-shadow',
          'background-color',
          'color',
          'font-size',
          'padding',
          'border-bottom',
          'line-height',
          'display',
          'gap',
        ].map((key) => [key, style.getPropertyValue(key)]),
      );
    };
    return {
      field: read(node.querySelector('.message-composer-field')!),
      status: read(node.querySelector('.annotation-status-row')!),
      send: read(node.querySelector('.annotation-status-row > button')!),
      structure: [...node.querySelectorAll('*')]
        .map((element) => ({ tag: element.tagName, className: element.className }))
        .filter((element) => element.tag !== 'P'),
    };
  });
}
for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const) {
    test(`identical composers and four review states on all surfaces: ${width} ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await page.addInitScript((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      const directory = process.env.COLAB_MENTION_CAPTURE_DIR ?? captureDirectory();
      mkdirSync(directory, { recursive: true });
      const proof: Record<string, unknown> = {};
      for (const state of ['default', 'multi', 'unknown', 'checking']) {
        let baseline: Awaited<ReturnType<typeof appearance>> | undefined;
        for (const surface of ['chat', 'thread', 'annotate']) {
          const window = await open(page, surface, state === 'checking');
          const composer = window.getByTestId('annotation-compose');
          const input = composer.getByRole('combobox', { name: 'Message', exact: true });
          await expect(composer).toBeVisible();
          if (state === 'checking') {
            await input.fill('@Agent 1 Explain.');
            await expect(composer.getByRole('status')).toHaveText('Checking for agents…');
            await expect(
              composer.getByRole('button', { name: 'Send', exact: true }),
            ).toBeDisabled();
          } else {
            await expect(input).toHaveText('@Agent 1 ', { useInnerText: true });
            if (state === 'multi') await input.pressSequentially('@Agent 2 Explain.');
            if (state === 'unknown') {
              await input.press('ControlOrMeta+A');
              await input.pressSequentially('@missing Explain.');
            }
            await expect(composer.getByRole('status')).toHaveText(
              state === 'default'
                ? 'Asks @Agent 1.'
                : state === 'multi'
                  ? 'Asks @Agent 1, @Agent 2.'
                  : 'No agent named @missing. This posts as a comment.',
            );
          }
          await expect(
            composer.getByRole('button', { name: 'Send', exact: true }),
          ).toBeInViewport();
          await expect(composer.locator('.tmt-ui-field-label')).toHaveCSS('width', '1px');
          await expect(input).toHaveCSS('border-radius', '0px');
          await expect(input).toHaveCSS('box-shadow', 'none');
          const current = await appearance(composer);
          if (baseline) expect(current).toEqual(baseline);
          else baseline = current;
          const name = `${surface}-${width}-${theme}-${state}`;
          proof[name] = current;
          await page.screenshot({ path: `${directory}/${name}.png` });
        }
      }
      writeFileSync(`${directory}/${width}-${theme}-parity.json`, JSON.stringify(proof, null, 2));
    });
  }
