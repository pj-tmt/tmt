import { test, expect, type Page, type Locator, type TestInfo } from '@playwright/test';
import { readFileSync, writeFileSync } from 'node:fs';

// Every scenario mounts the existing private foundation HTML; no product controllers.
const tokens = JSON.parse(
  readFileSync(new URL('../../../../../design/tokens/tokens.json', import.meta.url), 'utf8'),
) as {
  browser: {
    color: Record<string, Record<'light' | 'dark', string>>;
    surface: Record<string, Record<'light' | 'dark', string>>;
    metric: Record<string, string>;
    ui: Record<string, string>;
  };
};
type Theme = 'light' | 'dark';
type Kind = 'Dialog' | 'Sheet';
interface LayerState {
  dialogOpen: boolean;
  sheetOpen: boolean;
  menuOpen: boolean;
  popoverOpen: boolean;
  outsideActions: number;
  menuActions: number;
  denied: number;
  dialogEntries: number;
  sheetEntries: number;
  menuCloseFocusEvents: number;
  lastClose: string;
  mounted: boolean;
}
declare global {
  interface Window {
    __tmtLayerListenerSnapshot?: () => number[];
    __tmtLayerViewport?: {
      offsetLeft: number;
      offsetTop: number;
      width: number;
      height: number;
      dispatchEvent(event: Event): boolean;
    };
  }
}
const pageErrors = new WeakMap<Page, string[]>();
test.beforeEach(({ page }) => {
  const errors: string[] = [];
  pageErrors.set(page, errors);
  page.on('pageerror', (error) => errors.push(error.message));
});
test.afterEach(({ page }) => expect(pageErrors.get(page)).toEqual([]));

function rgb(hex: string): string {
  return `rgb(${[1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16)).join(', ')})`;
}
function fixture(page: Page): Locator {
  return page.locator('section[aria-label="Layer qualification fixture"]');
}
function surface(page: Page, kind: Kind): Locator {
  return page.getByRole('dialog', { name: `Fixture ${kind} title`, exact: true });
}
function trigger(page: Page, kind: Kind): Locator {
  return fixture(page).getByRole('button', { name: `Open fixture ${kind}`, exact: true });
}
async function state(page: Page): Promise<LayerState> {
  const value: unknown = JSON.parse(
    (await page.locator('[data-layer-state]').textContent()) ?? 'null',
  );
  if (typeof value !== 'object' || value === null) throw new Error('Missing private layer state');
  for (const key of ['dialogOpen', 'sheetOpen', 'menuOpen', 'popoverOpen', 'mounted'])
    if (!(key in value) || typeof Reflect.get(value, key) !== 'boolean')
      throw new Error(`Invalid fixture boolean ${key}`);
  for (const key of [
    'outsideActions',
    'menuActions',
    'denied',
    'dialogEntries',
    'sheetEntries',
    'menuCloseFocusEvents',
  ])
    if (!(key in value) || !Number.isSafeInteger(Reflect.get(value, key)))
      throw new Error(`Invalid fixture counter ${key}`);
  if (!('lastClose' in value) || typeof value.lastClose !== 'string')
    throw new Error('Invalid fixture close reason');
  return value as LayerState;
}
async function expectState(page: Page, expected: Partial<LayerState>): Promise<void> {
  await expect.poll(() => state(page)).toMatchObject(expected);
}
async function legacyStyles(page: Page) {
  return page.locator('#legacy [class*="tmt-ui"], #outside, #unscoped-input').evaluateAll((nodes) =>
    nodes.map((node) => {
      const style = getComputedStyle(node);
      return Object.fromEntries(
        [
          'color',
          'background-color',
          'font-family',
          'font-size',
          'padding',
          'margin',
          'border',
          'border-radius',
          'box-shadow',
          'outline',
          'box-sizing',
        ].map((key) => [key, style.getPropertyValue(key)]),
      );
    }),
  );
}
async function prepare(page: Page, width: number, theme: Theme): Promise<void> {
  await page.setViewportSize({ width, height: 900 });
  await page.emulateMedia({ colorScheme: theme });
  await page.goto('/test/browser-ui-foundation.html');
  await expect(fixture(page)).toBeVisible();
  const legacy = await legacyStyles(page);
  await page.getByRole('button', { name: 'Load isolated UI CSS', exact: true }).click();
  await expect(page.locator('[data-css-loaded=true]')).toHaveText('UI CSS loaded');
  await page.getByLabel('Outer theme').selectOption(theme);
  expect(await legacyStyles(page)).toEqual(legacy);
  await expectState(page, {
    dialogOpen: false,
    sheetOpen: false,
    menuOpen: false,
    popoverOpen: false,
    denied: 0,
    dialogEntries: 0,
    sheetEntries: 0,
    menuCloseFocusEvents: 0,
    outsideActions: 0,
    menuActions: 0,
  });
}
async function capture(page: Page, info: TestInfo, name: string): Promise<void> {
  const filename = `${name}.png`;
  const path = info.outputPath(filename);
  await page.screenshot({ path });
  await info.attach(name, { path, contentType: 'image/png' });
  const manifest = info.outputPath(`${name}-manifest.json`);
  writeFileSync(
    manifest,
    JSON.stringify({
      test: info.title,
      project: info.project.name,
      viewport: page.viewportSize(),
      capture: filename,
      state: await state(page),
      head: process.env.GITHUB_SHA ?? 'local exact head recorded by author',
    }),
  );
  await info.attach(`${name}-manifest`, {
    path: manifest,
    contentType: 'application/json',
  });
}
async function assertTokens(
  node: Locator,
  theme: Theme,
  role: 'paper' | 'sheet' = 'paper',
  radius: 'radius-l' | 'radius' = 'radius-l',
): Promise<void> {
  await expect(node).toHaveCSS('background-color', rgb(tokens.browser.surface[role][theme]));
  await expect(node).toHaveCSS('color', rgb(tokens.browser.color.text[theme]));
  await expect(node).toHaveCSS('border-color', rgb(tokens.browser.surface.rule[theme]));
  await expect(node).toHaveCSS('border-width', tokens.browser.metric['rule-width']);
  await expect(node).toHaveCSS('border-radius', tokens.browser.metric[radius]!);
  // A floating layer carries the one overlay shadow.
  await expect(node).toHaveCSS('box-shadow', /60px/);
  await expect(node).toHaveCSS('z-index', tokens.browser.ui['z-layer']);
  await expect(node).toHaveCSS('font-size', tokens.browser.metric['command-size']);
  expect(
    await node.evaluate(
      (element) =>
        !!element.closest('[data-tmt-ui-portal]') &&
        element.closest('[data-tmt-ui]')?.getAttribute('data-tmt-theme'),
    ),
  ).toBe(theme);
}
async function assertBounds(
  node: Locator,
  bounds: { left: number; top: number; width: number; height: number },
): Promise<void> {
  await expect
    .poll(async () => {
      const box = await node.boundingBox();
      return (
        box !== null &&
        box.x >= bounds.left - 1 &&
        box.y >= bounds.top - 1 &&
        box.x + box.width <= bounds.left + bounds.width + 1 &&
        box.y + box.height <= bounds.top + bounds.height + 1
      );
    })
    .toBe(true);
}
async function scrim(page: Page): Promise<void> {
  const viewport = page.viewportSize();
  if (!viewport) throw new Error('Missing private viewport');
  await page.mouse.click(viewport.width - 1, 1);
}
async function closeAndReturn(
  page: Page,
  kind: Kind,
  mode: 'Escape' | 'scrim' | 'action' | 'programmatic',
  opener: Locator,
): Promise<void> {
  if (mode === 'Escape') await page.keyboard.press('Escape');
  else if (mode === 'scrim') await scrim(page);
  else if (mode === 'action')
    await surface(page, kind).getByRole('button', { name: 'Close', exact: true }).last().click();
  else
    await surface(page, kind)
      .getByRole('button', { name: `Programmatic ${kind} close`, exact: true })
      .click();
  await expect(surface(page, kind)).toHaveCount(0);
  await expect(opener).toBeFocused();
}
async function installListenerProbe(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const add = EventTarget.prototype.addEventListener;
    const remove = EventTarget.prototype.removeEventListener;
    const entries: {
      target: EventTarget;
      type: string;
      listener: EventListenerOrEventListenerObject;
      capture: boolean;
      id: number;
    }[] = [];
    let next = 0;
    const captureFlag = (options?: boolean | AddEventListenerOptions | EventListenerOptions) =>
      typeof options === 'boolean' ? options : options?.capture === true;
    const observed = (target: EventTarget, type: string) =>
      (target === window || target === window.visualViewport) &&
      (type === 'resize' || type === 'scroll');
    EventTarget.prototype.addEventListener = function (type, listener, options) {
      if (
        listener &&
        observed(this, type) &&
        !entries.some(
          (item) =>
            item.target === this &&
            item.type === type &&
            item.listener === listener &&
            item.capture === captureFlag(options),
        )
      )
        entries.push({ target: this, type, listener, capture: captureFlag(options), id: next++ });
      add.call(this, type, listener, options);
    };
    EventTarget.prototype.removeEventListener = function (type, listener, options) {
      const index = entries.findIndex(
        (item) =>
          item.target === this &&
          item.type === type &&
          item.listener === listener &&
          item.capture === captureFlag(options),
      );
      if (index >= 0) entries.splice(index, 1);
      remove.call(this, type, listener, options);
    };
    window.__tmtLayerListenerSnapshot = () => entries.map((item) => item.id).sort((a, b) => a - b);
  });
}
async function listeners(page: Page): Promise<number[]> {
  return page.evaluate(() => {
    if (!window.__tmtLayerListenerSnapshot) throw new Error('Missing test-only listener probe');
    return window.__tmtLayerListenerSnapshot();
  });
}

for (const width of [320, 390, 1440])
  for (const theme of ['light', 'dark'] as const) {
    test.describe(`${width} ${theme} real layers`, () => {
      for (const kind of ['Dialog', 'Sheet'] as const) {
        test(`${kind} title entry, modal Tab, token geometry and all admitted close paths`, async ({
          page,
        }, info) => {
          await prepare(page, width, theme);
          await capture(page, info, `${kind}-closed`);
          for (const mode of ['Escape', 'scrim', 'action', 'programmatic'] as const) {
            const before = await state(page);
            await trigger(page, kind).click();
            const node = surface(page, kind);
            await expect(node).toBeVisible();
            await expect(
              node.getByRole('heading', { name: `Fixture ${kind} title`, exact: true }),
            ).toBeFocused();
            await assertTokens(node, theme);
            const scrimColor = await page.evaluate((token) => {
              const style = document.createElement('span').style;
              style.backgroundColor = token;
              return style.backgroundColor;
            }, tokens.browser.surface.scrim[theme]);
            await expect(page.locator(`[data-slot="${kind.toLowerCase()}-overlay"]`)).toHaveCSS(
              'background-color',
              scrimColor,
            );
            await assertBounds(node, { left: 0, top: 0, width, height: 900 });
            await expectState(page, {
              [kind === 'Dialog' ? 'dialogEntries' : 'sheetEntries']:
                before[kind === 'Dialog' ? 'dialogEntries' : 'sheetEntries'] + 1,
            });
            for (const key of ['Tab', 'Shift+Tab'])
              for (let i = 0; i < 8; i++) {
                await page.keyboard.press(key);
                expect(
                  await node.evaluate((element) => element.contains(document.activeElement)),
                ).toBe(true);
              }
            await capture(page, info, `${kind}-open-${mode}`);
            await closeAndReturn(page, kind, mode, trigger(page, kind));
            await expectState(page, {
              dialogOpen: false,
              sheetOpen: false,
              outsideActions: 0,
              menuActions: 0,
            });
          }
        });
        test(`menu-to-${kind} opens once, stays focused, and returns persistent trigger`, async ({
          page,
        }, info) => {
          await prepare(page, width, theme);
          const menuTrigger = fixture(page).getByRole('button', {
            name: 'Persistent menu trigger',
            exact: true,
          });
          for (const mode of ['Escape', 'scrim'] as const) {
            const before = await state(page);
            await menuTrigger.click();
            await page
              .getByRole('menuitem', { name: `Open ${kind} from menu`, exact: true })
              .click();
            await expectState(page, {
              menuOpen: false,
              menuCloseFocusEvents: before.menuCloseFocusEvents + 1,
              [kind === 'Dialog' ? 'dialogOpen' : 'sheetOpen']: true,
              [kind === 'Dialog' ? 'dialogEntries' : 'sheetEntries']:
                before[kind === 'Dialog' ? 'dialogEntries' : 'sheetEntries'] + 1,
            });
            await expect(
              surface(page, kind).getByRole('heading', {
                name: `Fixture ${kind} title`,
                exact: true,
              }),
            ).toBeFocused();
            // Observe the actual competing autofocus callback, not a timing workaround.
            await expect(page.getByRole('menu')).toHaveCount(0);
            await expect(
              surface(page, kind).getByRole('heading', {
                name: `Fixture ${kind} title`,
                exact: true,
              }),
            ).toBeFocused();
            await capture(page, info, `menu-to-${kind}-${mode}`);
            await closeAndReturn(page, kind, mode, menuTrigger);
            await expectState(page, { outsideActions: 0, menuActions: 0 });
          }
        });
        test(`${kind} busy/uncertain veto holds Escape, scrim, close and programmatic state`, async ({
          page,
        }, info) => {
          await prepare(page, width, theme);
          for (const held of ['Busy close veto', 'Uncertain close veto']) {
            await trigger(page, kind).click();
            const node = surface(page, kind);
            await node.getByLabel(held, { exact: true }).check();
            await expect(node).toHaveAttribute('aria-busy', 'true');
            await expect(
              node.getByText(held.startsWith('Busy') ? 'Busy' : 'Uncertain', { exact: true }),
            ).toBeVisible();
            if (kind === 'Dialog' && held.startsWith('Uncertain')) {
              const draft = node.getByLabel('Private draft', { exact: true });
              await expect(draft).toHaveAttribute('readonly', '');
              await draft.focus();
              await page.keyboard.type('must not change');
              await expect(draft).toHaveValue('Kept private draft');
            }
            for (const mode of ['Escape', 'scrim', 'action', 'programmatic'] as const) {
              const before = await state(page);
              if (mode === 'Escape') await page.keyboard.press('Escape');
              else if (mode === 'scrim') await scrim(page);
              else if (mode === 'action')
                await node.getByRole('button', { name: 'Close', exact: true }).last().click();
              else
                await node
                  .getByRole('button', { name: `Programmatic ${kind} close`, exact: true })
                  .click();
              await expectState(page, {
                denied: before.denied + 1,
                [kind === 'Dialog' ? 'dialogOpen' : 'sheetOpen']: true,
                outsideActions: 0,
                menuActions: 0,
              });
              await expect(node).toBeVisible();
              expect(
                await node.evaluate((element) => element.contains(document.activeElement)),
              ).toBe(true);
            }
            await capture(
              page,
              info,
              `${kind}-${held.startsWith('Busy') ? 'busy' : 'uncertain-readonly'}`,
            );
            await node.getByLabel(held, { exact: true }).uncheck();
            await closeAndReturn(page, kind, 'programmatic', trigger(page, kind));
          }
        });
      }
      test('all Sheet sides preserve token bounds, entry and return', async ({ page }, info) => {
        await prepare(page, width, theme);
        const gutter = Number.parseFloat(tokens.browser.ui['spacing-unit']) * 2;
        for (const side of ['top', 'right', 'bottom', 'left']) {
          await fixture(page).getByLabel('Sheet side', { exact: true }).selectOption(side);
          await trigger(page, 'Sheet').click();
          const node = surface(page, 'Sheet');
          await expect(node).toHaveAttribute('data-side', side);
          await expect(
            node.getByRole('heading', { name: 'Fixture Sheet title', exact: true }),
          ).toBeFocused();
          await assertTokens(node, theme);
          await assertBounds(node, { left: 0, top: 0, width, height: 900 });
          const box = await node.boundingBox();
          if (!box) throw new Error('Missing Sheet surface');
          const edge =
            side === 'left'
              ? box.x
              : side === 'right'
                ? width - box.x - box.width
                : side === 'top'
                  ? box.y
                  : 900 - box.y - box.height;
          expect(edge).toBeCloseTo(gutter, 0);
          await capture(page, info, `Sheet-side-${side}`);
          await closeAndReturn(page, 'Sheet', 'Escape', trigger(page, 'Sheet'));
        }
      });
      test('vetoed menu transfer never opens Dialog/Sheet or invokes the fake host action', async ({
        page,
      }, info) => {
        await prepare(page, width, theme);
        await fixture(page)
          .getByRole('button', { name: 'Persistent menu trigger', exact: true })
          .click();
        for (const held of ['Menu busy close veto', 'Menu uncertain close veto']) {
          const toggle = page.getByRole('menuitemcheckbox', { name: held, exact: true });
          await toggle.click();
          await expect(toggle).toHaveAttribute('aria-checked', 'true');
          for (const label of [
            'Open Dialog from menu',
            'Open Sheet from menu',
            'Admitted fixture action',
          ]) {
            const before = await state(page);
            await page.getByRole('menuitem', { name: label, exact: true }).click();
            await expectState(page, {
              denied: before.denied + 1,
              menuOpen: true,
              dialogOpen: false,
              sheetOpen: false,
              dialogEntries: 0,
              sheetEntries: 0,
              menuActions: 0,
              outsideActions: 0,
            });
            await expect(page.getByRole('dialog')).toHaveCount(0);
          }
          for (const key of ['Escape', 'Tab']) {
            const before = await state(page);
            await page.keyboard.press(key);
            await expectState(page, { denied: before.denied + 1, menuOpen: true });
          }
          const beforeOutside = await state(page);
          await scrim(page);
          await expectState(page, {
            denied: beforeOutside.denied + 1,
            menuOpen: true,
            outsideActions: 0,
          });
          await capture(
            page,
            info,
            held.startsWith('Menu busy') ? 'menu-busy-veto' : 'menu-uncertain-veto',
          );
          await toggle.click();
          await expect(toggle).toHaveAttribute('aria-checked', 'false');
        }
        await page.keyboard.press('Escape');
        await expectState(page, { menuOpen: false });
        await expect(
          fixture(page).getByRole('button', { name: 'Persistent menu trigger', exact: true }),
        ).toBeFocused();
      });
      test('menu keyboard, disabled items, stock submenu keys and first outside press', async ({
        page,
      }, info) => {
        await prepare(page, width, theme);
        const menuTrigger = fixture(page).getByRole('button', {
          name: 'Persistent menu trigger',
          exact: true,
        });
        await menuTrigger.focus();
        await page.keyboard.press('Enter');
        await expect(
          page.getByRole('menuitem', { name: 'Open Dialog from menu', exact: true }),
        ).toBeFocused();
        await assertTokens(page.getByRole('menu'), theme, 'sheet', 'radius');
        await assertBounds(page.getByRole('menu'), { left: 0, top: 0, width, height: 900 });
        await page.keyboard.press('ArrowDown');
        await expect(
          page.getByRole('menuitem', { name: 'Open Sheet from menu', exact: true }),
        ).toBeFocused();
        await page.keyboard.press('ArrowDown');
        await expect(
          page.getByRole('menuitem', { name: 'Admitted fixture action', exact: true }),
        ).toBeFocused();
        await page.keyboard.press('ArrowDown');
        await expect(
          page.getByRole('menuitem', { name: 'Unavailable action', exact: true }),
        ).toHaveAttribute('aria-disabled', 'true');
        await expect(
          page.getByRole('menuitemcheckbox', { name: 'Menu busy close veto', exact: true }),
        ).toBeFocused();
        const sub = page.getByRole('menuitem', { name: 'Nested choices', exact: true });
        await sub.focus();
        await page.keyboard.press('ArrowRight');
        await expect(
          page.getByRole('menuitem', { name: 'Nested item', exact: true }),
        ).toBeFocused();
        await page.keyboard.press('ArrowLeft');
        await expect(page.getByRole('menuitem', { name: 'Nested item', exact: true })).toHaveCount(
          0,
        );
        await expectState(page, { menuOpen: true });
        await expect(sub).toBeFocused();
        await page.keyboard.press('ArrowRight');
        await expect(
          page.getByRole('menuitem', { name: 'Nested item', exact: true }),
        ).toBeFocused();
        await page.keyboard.press('Escape');
        await expectState(page, { menuOpen: false });
        await expect(menuTrigger).toBeFocused();
        await menuTrigger.click();
        await capture(page, info, 'menu-disabled-and-nested');
        const beforeTabClose = await state(page);
        await page.keyboard.press('Tab');
        await expectState(page, {
          menuOpen: false,
          lastClose: 'tab',
          menuCloseFocusEvents: beforeTabClose.menuCloseFocusEvents + 1,
        });
        await expect(menuTrigger).toBeFocused();
        await menuTrigger.click();
        await expectState(page, { menuOpen: true });
        await expect(page.getByRole('menu')).toBeVisible();
        await expect(page.locator('body')).toHaveCSS('pointer-events', 'none');
        const underlying = fixture(page).getByRole('button', {
          name: 'Underlying host action',
          exact: true,
          // Radix's modal menu correctly hides background controls from AT.
          includeHidden: true,
        });
        const box = await underlying.boundingBox();
        const menuBox = await page.getByRole('menu').boundingBox();
        if (!box || !menuBox) throw new Error('Missing private outside-press geometry');
        // Platform fonts can put the menu over the underlying button's center.
        // Prove the chosen physical point is on that button and outside the menu.
        const point = [
          { x: box.x + 2, y: box.y + box.height / 2 },
          { x: box.x + box.width - 2, y: box.y + box.height / 2 },
          { x: box.x + box.width / 2, y: box.y + 2 },
          { x: box.x + box.width / 2, y: box.y + box.height - 2 },
        ].find(
          ({ x, y }) =>
            x >= 0 &&
            x < width &&
            y >= 0 &&
            y < 900 &&
            !(
              x >= menuBox.x &&
              x <= menuBox.x + menuBox.width &&
              y >= menuBox.y &&
              y <= menuBox.y + menuBox.height
            ),
        );
        if (!point) throw new Error('Underlying private control has no uncovered viewport point');
        const beforeOutside = await state(page);
        await page.mouse.click(point.x, point.y);
        await expectState(page, {
          menuOpen: false,
          lastClose: 'outside',
          outsideActions: 0,
          menuActions: 0,
          denied: beforeOutside.denied,
          menuCloseFocusEvents: beforeOutside.menuCloseFocusEvents + 1,
        });
        await underlying.click({ position: { x: point.x - box.x, y: point.y - box.y } });
        await expectState(page, { outsideActions: 1, menuActions: 0 });
      });
      test('nested stock nonmodal Popover loops Tab, permits outside focus and consumes inner Escape', async ({
        page,
      }, info) => {
        await prepare(page, width, theme);
        await trigger(page, 'Dialog').click();
        const parent = surface(page, 'Dialog');
        const opener = parent.getByRole('button', { name: 'Nested fixture Popover', exact: true });
        await opener.click();
        const popover = page.locator('[data-slot="popover-content"]');
        await expect(popover).toBeVisible();
        await assertTokens(popover, theme, 'sheet', 'radius');
        await assertBounds(popover, { left: 0, top: 0, width, height: 900 });
        await capture(page, info, 'nested-nonmodal-popover');
        await page.keyboard.press('Escape');
        await expectState(page, { popoverOpen: false, dialogOpen: true });
        await expect(opener).toBeFocused();
        await opener.click();
        await expect(popover).toBeVisible();
        const inside = popover.getByRole('button', {
          name: 'Programmatic Popover close',
          exact: true,
        });
        await expect(inside).toBeFocused();
        await page.keyboard.press('Tab');
        await expect(inside).toBeFocused();
        await page.keyboard.press('Shift+Tab');
        await expect(inside).toBeFocused();
        // Pinned Radix nonmodal Content loops Tab, but permits actual focus outside.
        await parent.getByRole('button', { name: 'Remove persistent opener', exact: true }).focus();
        await expectState(page, { popoverOpen: false, dialogOpen: true });
        // A leaving layer stays mounted until its exit has played and keeps the Escape: wait for it
        // to go before the next Escape closes the dialog.
        await expect(popover).toHaveCount(0);
        expect(await parent.evaluate((element) => element.contains(document.activeElement))).toBe(
          true,
        );
        await closeAndReturn(page, 'Dialog', 'Escape', trigger(page, 'Dialog'));
      });
      test('nonmodal Popover host veto holds Escape, pointer/focus outside and programmatic close', async ({
        page,
      }, info) => {
        await prepare(page, width, theme);
        await trigger(page, 'Dialog').click();
        const parent = surface(page, 'Dialog');
        const opener = parent.getByRole('button', { name: 'Nested fixture Popover', exact: true });
        for (const held of ['Busy close veto', 'Uncertain close veto']) {
          await parent.getByLabel(held, { exact: true }).check();
          await opener.click();
          const popup = page.locator('[data-slot="popover-content"]');
          await expect(popup).toBeVisible();
          const close = popup.getByRole('button', {
            name: 'Programmatic Popover close',
            exact: true,
          });
          for (const mode of ['escape', 'programmatic', 'pointer', 'focus']) {
            await close.focus();
            const before = await state(page);
            if (mode === 'escape') await page.keyboard.press('Escape');
            else if (mode === 'programmatic') await close.click();
            else if (mode === 'pointer')
              await parent
                .getByRole('heading', { name: 'Fixture Dialog title', exact: true })
                .click();
            else
              await parent
                .getByRole('button', { name: 'Remove persistent opener', exact: true })
                .focus();
            await expectState(page, {
              denied: before.denied + 1,
              dialogOpen: true,
              popoverOpen: true,
              outsideActions: 0,
              menuActions: 0,
            });
            await expect(popup).toBeVisible();
          }
          await capture(
            page,
            info,
            held.startsWith('Busy') ? 'popover-busy-veto' : 'popover-uncertain-veto',
          );
          await parent.getByLabel(held, { exact: true }).uncheck();
          // The outside checkbox click becomes an admitted pointer dismissal
          // once its host state releases the veto.
          await expectState(page, { popoverOpen: false, dialogOpen: true });
          await expect(opener).toBeFocused();
          await opener.click();
          await expect(popup).toBeVisible();
          await close.click();
          await expectState(page, { popoverOpen: false, dialogOpen: true });
          await expect(opener).toBeFocused();
        }
        await closeAndReturn(page, 'Dialog', 'Escape', trigger(page, 'Dialog'));
      });
      test('StrictMode repeated-open/unmount restores fallback and releases surface listeners/guards', async ({
        page,
      }, info) => {
        await installListenerProbe(page);
        await prepare(page, width, theme);
        const baseline = await listeners(page);
        const body = await page.locator('body').evaluate((element) => ({
          pointerEvents: element.style.pointerEvents,
          overflow: element.style.overflow,
        }));
        for (let cycle = 0; cycle < 3; cycle++) {
          await trigger(page, 'Dialog').click();
          await expect(surface(page, 'Dialog')).toBeVisible();
          expect((await listeners(page)).length).toBeGreaterThan(baseline.length);
          await closeAndReturn(page, 'Dialog', 'programmatic', trigger(page, 'Dialog'));
          await expect.poll(() => listeners(page)).toEqual(baseline);
          await expect(page.locator('[data-radix-focus-guard]')).toHaveCount(0);
        }
        await trigger(page, 'Dialog').click();
        await surface(page, 'Dialog')
          .getByRole('button', { name: 'Remove persistent opener', exact: true })
          .click();
        await closeAndReturn(
          page,
          'Dialog',
          'Escape',
          fixture(page).getByRole('button', { name: 'Persistent layer fallback', exact: true }),
        );
        await fixture(page)
          .getByRole('button', { name: 'Restore layer fixture', exact: true })
          .click();
        await trigger(page, 'Dialog').click();
        const before = await state(page);
        await surface(page, 'Dialog').getByLabel('Busy close veto', { exact: true }).check();
        await surface(page, 'Dialog')
          .getByRole('button', { name: 'Unmount fixture layers', exact: true })
          .click();
        await expectState(page, { mounted: false, dialogOpen: true, denied: before.denied });
        await expect(page.getByRole('dialog')).toHaveCount(0);
        await expect(
          fixture(page).getByRole('button', { name: 'Persistent layer fallback', exact: true }),
        ).toBeFocused();
        await expect.poll(() => listeners(page)).toEqual(baseline);
        await expect(page.locator('[data-radix-focus-guard]')).toHaveCount(0);
        expect(
          await page.locator('body').evaluate((element) => ({
            pointerEvents: element.style.pointerEvents,
            overflow: element.style.overflow,
          })),
        ).toEqual(body);
        await capture(page, info, 'raw-host-unmount-bypass');
      });
      test('stock Tooltip hover/focus/Escape is separate from actionable close veto', async ({
        page,
      }, info) => {
        await prepare(page, width, theme);
        await fixture(page).getByLabel('Busy close veto', { exact: true }).check();
        const opener = fixture(page).getByRole('button', {
          name: 'Stock fixture Tooltip',
          exact: true,
        });
        await opener.hover();
        const tooltip = page.locator('[data-slot="tooltip-content"]');
        await expect(tooltip).toBeVisible();
        await expect(page.getByRole('tooltip')).toHaveText(
          'Display-only help; stock Radix Escape.',
        );
        await expect(tooltip).toHaveCSS('background-color', rgb(tokens.browser.color.text[theme]));
        await expect(tooltip).toHaveCSS('color', rgb(tokens.browser.surface.paper[theme]));
        await expect(tooltip).toHaveCSS('z-index', tokens.browser.ui['z-tooltip']);
        await assertBounds(tooltip, { left: 0, top: 0, width, height: 900 });
        await capture(page, info, 'stock-tooltip-hover');
        await page.keyboard.press('Escape');
        await expect(tooltip).toHaveCount(0);
        await page.mouse.move(0, 0);
        await opener.focus();
        await expect(tooltip).toBeVisible();
        await page.keyboard.press('Escape');
        await expect(tooltip).toHaveCount(0);
        await expect(opener).toBeFocused();
        await expectState(page, {
          denied: 0,
          dialogOpen: false,
          sheetOpen: false,
          menuOpen: false,
        });
      });
      test('viewport-offset/reduced-height simulation bounds layers after page/header scroll', async ({
        page,
      }, info) => {
        await prepare(page, width, theme);
        await page.evaluate(() => {
          const viewport = Object.assign(new EventTarget(), {
            offsetLeft: 0,
            offsetTop: 0,
            pageLeft: 0,
            pageTop: 0,
            width: window.innerWidth,
            height: window.innerHeight,
            scale: 1,
          });
          window.__tmtLayerViewport = viewport;
          Object.defineProperty(window, 'visualViewport', { configurable: true, value: viewport });
          document
            .querySelector('#legacy')
            ?.setAttribute('style', 'overflow: hidden; height: 80px');
          const spacer = document.createElement('div');
          spacer.style.height = '900px';
          document.body.append(spacer);
          window.scrollTo(0, 80);
        });
        for (const kind of ['Dialog', 'Sheet'] as const) {
          await trigger(page, kind).evaluate((element) =>
            element.scrollIntoView({ block: 'center' }),
          );
          expect(await page.evaluate(() => window.scrollY)).toBeGreaterThan(0);
          await trigger(page, kind).click();
          await page.evaluate(() => {
            const viewport = window.__tmtLayerViewport;
            if (!viewport) throw new Error('Missing test-only viewport');
            viewport.offsetLeft = 8;
            viewport.width = window.innerWidth - 16;
            viewport.offsetTop = 170;
            viewport.height = 420;
            viewport.dispatchEvent(new Event('resize'));
            viewport.dispatchEvent(new Event('scroll'));
            window.dispatchEvent(new Event('scroll'));
          });
          const node = surface(page, kind);
          await assertBounds(node, { left: 8, top: 170, width: width - 16, height: 420 });
          await expect(
            node.getByRole('heading', { name: `Fixture ${kind} title`, exact: true }),
          ).toBeInViewport();
          await capture(page, info, `${kind}-simulated-soft-keyboard`);
          await closeAndReturn(page, kind, 'programmatic', trigger(page, kind));
        }
      });
    });
  }
