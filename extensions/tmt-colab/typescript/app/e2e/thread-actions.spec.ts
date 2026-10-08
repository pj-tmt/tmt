import { expect, test, type Page } from '@playwright/test';
import { mkdirSync, writeFileSync } from 'node:fs';

const fixture = '/test/annotation-browser.tsx';
async function run(page: Page, method: string, argument?: string) {
  return page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
}
const before = process.env.COLAB_1804B_CAPTURE_PHASE === 'before';
for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const) {
    test(`thread actions ${width}px ${theme}: status, tooltip and busy fences`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await page.addInitScript((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      await page.goto('/');
      const window = page.getByTestId('comment-thread');
      const capture = async (state: string) => {
        await page.mouse.move(width - 1, 899);
        const directory = process.env.COLAB_1804B_CAPTURE_DIR;
        if (!directory) return;
        mkdirSync(directory, { recursive: true });
        const name = `${width}-${theme}-${state}`;
        await page.screenshot({ path: `${directory}/${name}.png` });
        writeFileSync(
          `${directory}/${name}.json`,
          JSON.stringify(
            await window.evaluate((node) => ({
              title: node.querySelector('.thread-state')?.textContent,
              header: node.querySelector('.thread-bar')?.getBoundingClientRect().toJSON(),
              actions: node.querySelector('.thread-bar-actions')?.getBoundingClientRect().toJSON(),
              quote: node.querySelector('blockquote')?.getBoundingClientRect().toJSON(),
              scrollTop: node.querySelector('.thread-messages')?.scrollTop,
              focus: document.activeElement?.getAttribute('aria-label'),
              buttons: Array.from(node.querySelectorAll('button')).map((button) => ({
                label: button.getAttribute('aria-label') || button.textContent,
                disabled: button.disabled,
                busy: button.getAttribute('aria-busy'),
                title: button.getAttribute('title'),
                rect: button.getBoundingClientRect().toJSON(),
              })),
            })),
            null,
            2,
          ),
        );
      };
      for (const state of ['open', 'agent', 'person', 'disabled']) {
        await run(page, 'mountWindow', `capture-${state}`);
        await expect(window).toBeVisible();
        const title = window.locator('.thread-state');
        await expect(title).toHaveText(
          state === 'agent'
            ? 'Resolved by Release coordination assistant'
            : state === 'person'
              ? 'Resolved'
              : 'Open',
        );
        if (!before) {
          const actions = window.locator('.thread-bar-actions');
          const titleRect = (await title.boundingBox())!;
          const actionRect = (await actions.boundingBox())!;
          expect(titleRect.x + titleRect.width).toBeLessThanOrEqual(actionRect.x);
          expect(
            Math.abs(titleRect.y + titleRect.height / 2 - actionRect.y - actionRect.height / 2),
          ).toBeLessThan(2);
          await expect(actions.locator('button')).toHaveCount(2);
          for (const button of await actions.locator('button').all())
            await expect(button).toHaveClass(/tmt-ui-icon-action-control/);
          expect(await actions.locator('button[title]').count()).toBe(0);
          await expect(
            window.getByRole('button', { name: 'Delete thread', exact: true }),
          ).toHaveAttribute('data-variant', 'text');
          await expect(
            page.getByRole('button', { name: 'Post reply', exact: true }),
          ).toHaveAttribute('data-variant', 'primary');
          const label = window
            .getByRole('button', { name: 'Delete thread', exact: true })
            .locator('.tmt-ui-action-label');
          expect(
            Math.abs((await label.boundingBox())!.x - (await title.boundingBox())!.x),
          ).toBeLessThan(1);
        }
        await capture(state);
        if (state === 'open') {
          const history = window.locator('.thread-messages');
          await history.evaluate((node) => {
            node.scrollTop = 0;
          });
          const quote = history.locator('blockquote');
          await expect(quote).toHaveText('Frozen original quote');
          const quoteRect = (await quote.boundingBox())!;
          const historyRect = (await history.boundingBox())!;
          const headerRect = (await window.locator('.thread-bar').boundingBox())!;
          expect(quoteRect.y).toBeGreaterThanOrEqual(historyRect.y);
          expect(quoteRect.y).toBeGreaterThan(headerRect.y + headerRect.height);
          expect(quoteRect.y + quoteRect.height).toBeLessThanOrEqual(
            historyRect.y + historyRect.height,
          );
          expect(await history.evaluate((node) => node.scrollTop)).toBe(0);
          await capture('quote-top');
        }
        if (state === 'agent') {
          const reopen = window.getByRole('button', { name: 'Reopen', exact: true });
          await reopen.focus();
          await page.keyboard.press('Tab');
          await page.keyboard.press('Shift+Tab');
          if (!before)
            await expect(reopen.locator('..').locator('.tmt-ui-icon-action-tooltip')).toBeVisible();
          await capture('focus-reopen');
        }
        if (state === 'disabled') {
          await expect(window.getByRole('button', { name: 'Resolve', exact: true })).toBeDisabled();
          await expect(
            window.getByRole('button', { name: 'Delete thread', exact: true }),
          ).toBeDisabled();
          expect(await run(page, 'windowProof')).toEqual({ statusCalls: 0, closes: 0 });
        }
      }
      await run(page, 'mountWindow', 'capture-open');
      const input = page.getByRole('combobox', { name: 'Message', exact: true });
      await input.fill('Reply draft with a retained caret.');
      await capture('reply');
      for (const label of ['Resolve', 'Close thread']) {
        const button = window.getByRole('button', { name: label, exact: true });
        await button.focus();
        await page.keyboard.press('Tab');
        await page.keyboard.press('Shift+Tab');
        await expect(button).toBeFocused();
        if (!before) {
          const tooltip = button.locator('..').locator('.tmt-ui-icon-action-tooltip');
          await expect(tooltip).toBeVisible();
          await expect(tooltip).toHaveText(label);
          await page.keyboard.press('Escape');
          await expect(tooltip).toBeHidden();
          expect(await run(page, 'windowProof')).toEqual({ statusCalls: 0, closes: 0 });
          await page.keyboard.press('Tab');
          await page.keyboard.press('Shift+Tab');
          await expect(tooltip).toBeVisible();
        }
        await capture(`focus-${label === 'Resolve' ? 'resolve' : 'close'}`);
      }
      await window.getByRole('button', { name: 'Resolve', exact: true }).click();
      await expect(window.getByRole('button', { name: 'Resolve', exact: true })).toBeDisabled();
      await expect(
        window.getByRole('button', { name: 'Close thread', exact: true }),
      ).toBeDisabled();
      await expect(
        window.getByRole('button', { name: 'Delete thread', exact: true }),
      ).toBeDisabled();
      if (!before)
        await expect(window.getByRole('button', { name: 'Resolve', exact: true })).toHaveAttribute(
          'aria-busy',
          'true',
        );
      await capture('busy');
      expect(await run(page, 'windowProof')).toEqual({ statusCalls: 1, closes: 0 });
      await run(page, 'finishWindowStatus', 'failed');
      await expect(window.getByRole('alert')).toBeVisible();
      await expect(input).toHaveText('Reply draft with a retained caret.', { useInnerText: true });
    });
    test(`thread submits ${width}px ${theme}: visible busy feedback and retained failed draft`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await page.addInitScript((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      await page.goto('/');
      for (const kind of ['post', 'edit']) {
        await run(page, 'mountWindow', kind === 'post' ? 'capture-compose' : 'capture-open');
        if (kind === 'post')
          await page.getByRole('button', { name: '+ Comment on page', exact: true }).click();
        else {
          await page.getByRole('button', { name: 'Message actions', exact: true }).click();
          await page.getByRole('menuitem', { name: 'Edit', exact: true }).click();
        }
        const input = page.getByRole('combobox', {
          name: kind === 'post' ? 'Post comment' : 'Edit comment',
          exact: true,
        });
        const draft = `Retain this ${kind} draft on failure.`;
        await input.fill(draft);
        const form = input.locator('xpath=ancestor::form');
        const submit = form.locator('.comment-actions > button:not([type="button"])');
        await submit.click();
        await expect(submit).toBeDisabled();
        if (!before) {
          await expect(submit).toHaveAttribute('aria-busy', 'true');
          await expect(
            submit.locator('.tmt-ui-action-mark[data-busy="true"] .lucide-loader-circle'),
          ).toBeVisible();
        }
        expect(await run(page, 'windowWriteProof')).toEqual({ writes: 1, captured: [draft] });
        const directory = process.env.COLAB_1804B_CAPTURE_DIR;
        if (directory) {
          mkdirSync(directory, { recursive: true });
          await input.scrollIntoViewIfNeeded();
          await page.screenshot({ path: `${directory}/${width}-${theme}-submit-${kind}-busy.png` });
        }
        await run(page, 'finishWindowWrite', 'failed');
        await expect(page.getByRole('alert')).toBeVisible();
        await expect(input).toHaveText(draft, { useInnerText: true });
        await expect(submit).toBeEnabled();
        if (!before) await expect(submit.locator('.tmt-ui-action-mark')).toBeHidden();
        expect(await run(page, 'windowWriteProof')).toEqual({ writes: 1, captured: [draft] });
      }
    });
    test(`page icon actions ${width}px ${theme}: disclosure keeps host Escape`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await page.addInitScript((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      await page.goto('/');
      await page.evaluate(async () => {
        const path = '/test/page-layout-browser.tsx';
        (await import(path)).mount(
          '<h1>Review page</h1><p>Conversation tools stay beside the page.</p>',
        );
      });
      await expect(page.frameLocator('iframe').locator('h1')).toBeVisible();
      const capture = async (state: string) => {
        await page.mouse.move(width - 1, 899);
        const directory = process.env.COLAB_1804B_CAPTURE_DIR;
        if (!directory) return;
        await page.screenshot({ path: `${directory}/${width}-${theme}-header-${state}.png` });
      };
      const focus = async (name: string) => {
        const button = page.getByRole('button', { name, exact: true });
        await button.focus();
        await page.keyboard.press('Tab');
        await page.keyboard.press('Shift+Tab');
        await expect(button).toBeFocused();
        if (!before) {
          await expect(button).toHaveClass(/tmt-ui-icon-action-control/);
          await expect(button.locator('..').locator('.tmt-ui-icon-action-tooltip')).toBeVisible();
        }
        return button;
      };
      if (width === 390) {
        const more = await focus('More page actions');
        await capture('focus-more');
        await more.click();
        await expect(more).toHaveAttribute('aria-expanded', 'true');
        if (!before)
          await expect(more.locator('..').locator('.tmt-ui-icon-action-tooltip')).toBeHidden();
        await capture('menu');
        // The expanded trigger owns no tooltip listener: one Escape closes the host menu.
        await page.keyboard.press('Escape');
        await expect(more).toHaveAttribute('aria-expanded', 'false');
        await expect(more).toBeFocused();
        await more.click();
        await focus('Close page actions');
        await capture('focus-close');
        await page.keyboard.press('Escape');
        if (!before) {
          await expect(more).toHaveAttribute('aria-expanded', 'true');
          await page.keyboard.press('Escape');
        }
        await expect(more).toHaveAttribute('aria-expanded', 'false');
        await more.click();
      }
      const themeButton = await focus('Change color theme');
      await capture('focus-theme');
      await themeButton.click();
      await expect(page.locator('html')).toHaveAttribute(
        'data-theme',
        theme === 'dark' ? 'light' : 'dark',
      );
    });
  }
