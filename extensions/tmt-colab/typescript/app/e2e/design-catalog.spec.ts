import { browserElements } from '@tmt/browser-ui/elements-manifest';
import { readFileSync } from 'node:fs';
import { expect, test } from '@playwright/test';
import { openModuleNavigation } from '../test/home-list.js';
const fixture = '/test/ask-page-browser.tsx';
for (const width of [1440, 390])
  for (const theme of ['light', 'dark']) {
    test(`Question draft survives a live toggle ${width} ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 1000 });
      await page.goto('/');
      await page.evaluate(
        async ({ fixture, theme }) => {
          document.documentElement.dataset.theme = theme;
          const owner = await import(fixture);
          await owner.mount();
          await owner.change(
            '<h1>Widget question</h1><tmt-ui-toggle label="Follow updates"></tmt-ui-toggle><tmt-question data-id="00000000-0000-4000-8000-000000000090" data-kind="short" data-agent="Agent 1"><p data-question>Keep this question draft?</p></tmt-question>',
          );
        },
        { fixture, theme },
      );
      const frame = page.frameLocator('#ask-page-fixture iframe');
      await expect(frame.getByRole('heading', { name: 'Widget question' })).toBeVisible();
      await page.locator('.question-group-card .question-line').click();
      const answer = page
        .locator('.question-group-card')
        .getByRole('textbox', { name: 'Your answer' });
      await answer.fill('Retained beside the live widget');
      const toggle = frame.getByRole('button', { name: 'Follow updates' });
      await toggle.click();
      await expect(toggle).toHaveAttribute('aria-pressed', 'true');
      const paint = await toggle.evaluate((node) => {
        const sample = document.createElement('span');
        sample.style.backgroundColor = 'var(--tmt-ui-surface-selection)';
        document.body.append(sample);
        const expected = getComputedStyle(sample).backgroundColor;
        sample.remove();
        return {
          actual: getComputedStyle(node).backgroundColor,
          expected,
          annotation: CSS.highlights?.has('colab-selected') ?? false,
        };
      });
      expect(paint.actual).toBe(paint.expected);
      expect(paint.annotation).toBe(false);
      await expect(answer).toHaveValue('Retained beside the live widget');
      expect(
        await page.evaluate(async (fixture) => (await import(fixture)).proof(), fixture),
      ).toEqual({ sends: [], actions: [] });
    });
    for (const surface of ['index', 'detail', 'terminal', 'document']) {
      test(`Design ${surface} ${width} ${theme}`, async ({ page }, info) => {
        await page.setViewportSize({ width, height: 1000 });
        await page.route('**/src/main.tsx', (route) =>
          route.fulfill({ contentType: 'text/javascript', body: '' }),
        );
        await page.goto('/');
        await page.evaluate(
          async ({ fixture, theme }) => {
            document.documentElement.dataset.theme = theme;
            await (await import(fixture)).mount();
          },
          { fixture, theme },
        );
        if (surface === 'document') {
          await page.evaluate(async (fixture) => {
            const path = '/src/design-document.ts';
            const { designDocument } = await import(path);
            await (await import(fixture)).change(designDocument, 'Shortcut dialog — proposal');
          }, fixture);
          const frame = page.frameLocator('#ask-page-fixture iframe');
          await expect(
            frame.getByRole('heading', { name: 'Shortcut dialog — proposal' }),
          ).toBeVisible();
          await expect(frame.getByRole('tablist', { name: 'Category' })).toBeVisible();
          await info.attach(`design-proposal-${width}-${theme}-closed`, {
            body: await page.screenshot(),
            contentType: 'image/png',
          });
          await frame.getByRole('button', { name: /Sort/ }).click();
          await frame.getByRole('menuitemradio', { name: 'Title', exact: true }).click();
          await expect(frame.getByRole('button', { name: /Sort/ })).toContainText('Title');
          await frame.getByRole('tab', { name: 'Needs you', exact: true }).click();
          await expect(frame.getByRole('tabpanel', { name: 'Needs you' })).toBeVisible();
          const trigger = frame.getByRole('button', { name: 'Manage shortcuts', exact: true });
          await trigger.click();
          const dialog = frame.getByRole('dialog', { name: 'Manage shortcuts' });
          const name = dialog.getByRole('textbox', { name: 'Name', exact: true });
          await name.fill('Design reviews draft');
          const checkbox = dialog.getByRole('checkbox', { name: 'Follow updates' });
          await checkbox.check();
          await expect(checkbox).toBeChecked();
          expect(
            await checkbox.evaluate((node) => ({
              background: getComputedStyle(node.closest('label')!).backgroundColor,
              annotation: CSS.highlights?.has('colab-selected') ?? false,
            })),
          ).toEqual({ background: 'rgba(0, 0, 0, 0)', annotation: false });
          await dialog.press('Escape');
          await expect(trigger).toBeFocused();
          await trigger.click();
          await expect(name).toHaveValue('Design reviews draft');
          await expect(checkbox).toBeChecked();
          await name.focus();
          expect(
            await page.evaluate(async (fixture) => (await import(fixture)).proof(), fixture),
          ).toEqual({ sends: [], actions: [] });
        } else {
          const path =
            surface === 'detail'
              ? '/design/gui/tmt-ui-dialog'
              : surface === 'terminal'
                ? '/design/tui'
                : '/design/gui';
          // The real app router owns these routes independently of the menu entry.
          await page.evaluate((path) => {
            location.hash = path;
          }, path);
          const catalog = page.locator('.design-page');
          await expect(catalog).toBeVisible();
          const currentTitle = page.locator('.tmt-ui-topbar-title-current');
          const prefix = page.locator('.tmt-ui-topbar-title-prefix');
          await expect(currentTitle).toHaveText(
            surface === 'terminal' ? 'TUI' : surface === 'detail' ? 'Dialog' : 'GUI',
          );
          if (width === 390) await expect(prefix).toBeHidden();
          else {
            await expect(prefix).toBeVisible();
            const spacing = await prefix.evaluate((node) => {
              const groups = node.querySelectorAll('.colab-crumb-group');
              return {
                gap:
                  groups[1]!.getBoundingClientRect().left -
                  groups[0]!.getBoundingClientRect().right,
                token: parseFloat(getComputedStyle(node).getPropertyValue('--tmt-ui-header-gap')),
              };
            });
            expect(spacing.token).toBeGreaterThan(0);
            expect(spacing.gap).toBeGreaterThanOrEqual(spacing.token);
            expect(
              Number(await prefix.evaluate((node) => getComputedStyle(node).fontWeight)),
            ).toBeLessThan(
              Number(await currentTitle.evaluate((node) => getComputedStyle(node).fontWeight)),
            );
          }
          if (surface === 'terminal') {
            await expect(catalog.getByRole('table', { name: 'TUI components' })).toBeVisible();
            await expect(catalog).toContainText('not available in this build');
            await expect(catalog.locator('iframe')).toHaveCount(0);
          } else {
            const previews = catalog.locator('[data-design-ready]');
            await expect(previews).toHaveCount(
              surface === 'index' ? browserElements.length + 1 : 1,
            );
            await expect(catalog.getByRole('alert')).toHaveCount(0);
            if (surface === 'index') {
              const menu = await openModuleNavigation(page);
              await expect(
                menu
                  .getByRole('list', { name: 'Design system', exact: true })
                  .getByRole('link', { name: 'Design', exact: true }),
              ).toHaveAttribute('aria-current', 'page');
              await expect(
                menu
                  .getByRole('list', { name: 'Shortcuts', exact: true })
                  .getByRole('link', { name: 'Design', exact: true }),
              ).toHaveCount(0);
              if (width === 390) {
                const opener = page.getByRole('button', { name: 'Open navigation', exact: true });
                await opener.press('Escape');
                await expect(opener).toHaveAttribute('aria-expanded', 'false');
              }
              await expect(catalog.getByRole('heading', { name: 'GUI', exact: true })).toHaveCount(
                0,
              );
              await expect(catalog.getByRole('tablist', { name: 'Design systems' })).toBeVisible();
              const filters = catalog.getByRole('tablist', { name: 'Component categories' });
              await expect(
                filters.getByRole('tab', { name: `All, ${browserElements.length}` }),
              ).toHaveAttribute('aria-selected', 'true');
              for (const category of new Set(browserElements.map((entry) => entry.category))) {
                const count = browserElements.filter((entry) => entry.category === category).length;
                await expect(
                  filters.getByRole('tab', { name: `${category}, ${count}`, exact: true }),
                ).toBeVisible();
              }
              const navigationCount = browserElements.filter(
                (entry) => entry.category === 'Navigation',
              ).length;
              await filters
                .getByRole('tab', { name: `Navigation, ${navigationCount}`, exact: true })
                .click();
              await expect(catalog.locator('.design-grid > article')).toHaveCount(navigationCount);
              await filters
                .getByRole('tab', { name: `All, ${browserElements.length}`, exact: true })
                .click();
              const systems = catalog.getByRole('tablist', { name: 'Design systems' });
              await systems.getByRole('tab', { name: 'TUI', exact: true }).click();
              await expect(currentTitle).toHaveText('TUI');
              await systems.getByRole('tab', { name: 'GUI', exact: true }).click();
              await expect(currentTitle).toHaveText('GUI');
              await expect(previews).toHaveCount(browserElements.length + 1);
              const lifetimes = [];
              for (const entry of browserElements) {
                const preview = catalog.getByRole('region', {
                  name: `${entry.name} live preview`,
                  exact: true,
                });
                const frame = preview.frameLocator('iframe');
                lifetimes.push(
                  await frame.locator('body').evaluate((body) => ({
                    readyState: document.readyState,
                    widgets: [...body.querySelectorAll('*')]
                      .filter((node) => node.localName.startsWith('tmt-ui-'))
                      .map((node) => ({
                        tag: node.localName,
                        registered: !!customElements.get(node.localName),
                        children: node.childElementCount,
                        controls: node.querySelectorAll(
                          'button,input,[role="tablist"],.tmt-ui-notice,.tmt-ui-chip,.tmt-ui-list-table',
                        ).length,
                      })),
                  })),
                );
              }
              await info.attach('preview-lifetimes', {
                body: JSON.stringify(lifetimes, null, 2),
                contentType: 'application/json',
              });
              for (const entry of browserElements) {
                const preview = catalog.getByRole('region', {
                  name: `${entry.name} live preview`,
                  exact: true,
                });
                await expect(
                  preview
                    .frameLocator('iframe')
                    .locator(
                      'button,input,[role="tablist"],.tmt-ui-notice,.tmt-ui-chip,.tmt-ui-list-table',
                    )
                    .first(),
                  `${entry.tag} must finish upgrading in its real catalog frame`,
                ).toBeVisible();
                await preview.scrollIntoViewIfNeeded();
                await expect
                  .poll(async () => {
                    const content = await preview
                      .frameLocator('iframe')
                      .locator('body')
                      .evaluate((body) =>
                        Math.ceil(
                          body.getBoundingClientRect().bottom +
                            parseFloat(getComputedStyle(body).marginBottom),
                        ),
                      );
                    const outer = await preview
                      .locator('iframe')
                      .evaluate((node) => node.getBoundingClientRect().height);
                    return outer >= Math.max(72, content);
                  })
                  .toBe(true);
                const height = await preview
                  .locator('iframe')
                  .evaluate((node) => node.getBoundingClientRect().height);
                expect(height).toBeGreaterThanOrEqual(72);
                expect(height).toBeLessThan(240);
                await info.attach(`design-preview-${entry.tag}-${width}-${theme}`, {
                  body: await preview.screenshot(),
                  contentType: 'image/png',
                });
              }
              await catalog
                .getByRole('tablist', { name: 'Design systems', exact: true })
                .scrollIntoViewIfNeeded();
              // Returning from the last offscreen preview needs the visible iframe painters, too.
              const visible = catalog.locator('.design-grid iframe');
              for (const iframe of await visible.all()) {
                const box = await iframe.boundingBox();
                if (box && box.y + box.height > 0 && box.y < 1000)
                  await iframe
                    .contentFrame()
                    .locator('html')
                    .evaluate(
                      () =>
                        new Promise<void>((resolve) =>
                          requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
                        ),
                    );
              }
            }

            if (surface === 'detail') {
              const frame = catalog.frameLocator('iframe');
              const trigger = frame
                .getByRole('button', { name: 'Manage shortcuts', exact: true })
                .first();
              expect(
                await trigger.evaluate((node) => parseFloat(getComputedStyle(node).paddingTop)),
              ).toBeGreaterThan(0);
              await trigger.click();
              await expect(frame.getByRole('dialog').first()).toBeVisible();
              await frame.getByRole('dialog').first().press('Escape');
            }
            await expect(catalog.getByLabel('Indexed coverage').first()).toContainText('✓ Test');
          }
        }
        await info.attach(`design-${surface}-${width}-${theme}`, {
          body: await page.screenshot({ fullPage: surface !== 'index' }),
          contentType: 'image/png',
        });
      });
    }
  }

test('widget runtime admission matches the shared native export vectors', async ({ page }) => {
  const { cases } = JSON.parse(
    readFileSync(
      new URL('../../../contracts/vectors/runtime-marker-v1.json', import.meta.url),
      'utf8',
    ),
  ) as { cases: { name: string; source: string; optsIn: boolean }[] };
  await page.goto('/');
  const answers = await page.evaluate(
    async (sources) => {
      const path = '/src/page-runtime.ts';
      const { wantsRuntime } = await import(path);
      return sources.map((source: string) => wantsRuntime(source));
    },
    cases.map((entry) => entry.source),
  );
  expect(cases.map((entry, index) => [entry.name, answers[index]])).toEqual(
    cases.map((entry) => [entry.name, entry.optsIn]),
  );
});

test('reader loads script-free widgets and remounts replaced widget source', async ({ page }) => {
  const fixture = '/test/reader-metadata-browser.tsx';
  await page.goto('/');
  await page.evaluate(async (fixture) => {
    const owner = await import(fixture);
    await owner.mount('viewer');
    owner.source('<h1>Reader widgets</h1><tmt-ui-toggle label="Follow updates"></tmt-ui-toggle>');
  }, fixture);
  const frame = page.frameLocator('#reader-metadata-fixture iframe');
  const control = frame.getByRole('button', { name: 'Follow updates' });
  await control.click();
  await expect(control).toHaveAttribute('aria-pressed', 'true');
  const previous = await page.locator('#reader-metadata-fixture iframe').elementHandle();
  await page.evaluate(
    async (fixture) =>
      (await import(fixture)).source(
        '<h1>Changed reader widgets</h1><tmt-ui-toggle label="Follow updates"></tmt-ui-toggle>',
      ),
    fixture,
  );
  await expect(frame.getByRole('heading', { name: 'Changed reader widgets' })).toBeVisible();
  expect(await previous!.evaluate((node) => node.isConnected)).toBe(false);
  await expect(control).toHaveAttribute('aria-pressed', 'false');
});
