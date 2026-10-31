import { expect, test, type Page } from '@playwright/test';
import { capturePath } from './captures.js';
import { pageAction } from '../test/page-actions.js';
const fixture = '/test/inline-slots-browser.tsx';
for (const occupied of [249, 250])
  test(`reading slots preserve all ${occupied} existing controls within the shared cap`, async ({
    page,
  }) => {
    await page.goto('/');
    await page.evaluate(
      async ({ fixture, occupied }) => (await import(fixture)).mount({ occupied, readings: 2 }),
      { fixture, occupied },
    );
    const proof = () => page.evaluate(async (fixture) => (await import(fixture)).proof(), fixture);
    await expect.poll(async () => (await proof())?.length).toBe(250);
    const slots = await proof();
    expect(slots.slice(0, occupied).map((slot: { id: string }) => slot.id)).toEqual(
      Array.from(
        { length: occupied },
        (_, index) => `10000000-0000-4000-8000-${String(index).padStart(12, '0')}`,
      ),
    );
    expect(slots.slice(occupied).map((slot: { id: string }) => slot.id)).toEqual(
      occupied === 249 ? ['10000000-0000-4000-8000-000000001000'] : [],
    );
  });

test('a reading ID collision keeps the original control and refuses only the reading slot', async ({
  page,
}) => {
  await page.goto('/');
  await page.evaluate(
    async (fixture) => (await import(fixture)).mount({ occupied: 1, readings: 1, collide: true }),
    fixture,
  );
  await expect
    .poll(async () => page.evaluate(async (fixture) => (await import(fixture)).proof(), fixture))
    .toEqual([{ id: '10000000-0000-4000-8000-000000000000', height: 2 }]);
  await expect(page.locator('[data-selected-readings]')).toHaveText('');
});

async function fixtureCall(page: Page, name: string, method: string, argument?: unknown) {
  return page.evaluate(
    async ({ name, method, argument }) => (await import(`/test/${name}.tsx`))[method](argument),
    { name, method, argument },
  );
}
async function flushResize(page: Page) {
  await page.evaluate(
    () =>
      new Promise<void>((resolve) =>
        requestAnimationFrame(() =>
          requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
        ),
      ),
  );
}
for (const initiallyHidden of [false, true])
  test(`hidden measurements retain request kinds without a request loop (initially hidden: ${initiallyHidden})`, async ({
    page,
  }) => {
    await page.goto('/');
    const call = (method: string, argument?: unknown) =>
      fixtureCall(page, 'inline-slots-browser', method, argument);
    await call('mountVisibility', initiallyHidden);
    await expect.poll(async () => (await call('proof'))?.length).toBe(4);
    const hidden = await call('proof');
    expect(Object.keys(hidden[0]).sort()).toEqual(['height', 'id']);
    expect(hidden[1]).toHaveProperty('before');
    expect(hidden[2]).toHaveProperty('chip');
    expect(Object.keys(hidden[3]).sort()).toEqual(['height', 'id']);
    if (initiallyHidden) await call('setVisibility', false);
    await expect(page.getByLabel('Answer draft')).toBeVisible();
    await page.getByLabel('Answer draft').fill('An unsent answer');
    await page.getByLabel('Ordinary draft').fill('An unsent proposal');
    await flushResize(page);
    const before = await call('proof');
    const count = await call('batchCount');
    await call('setVisibility', true);
    await expect(page.getByLabel('Answer draft')).toBeHidden();
    await flushResize(page);
    expect(await call('proof')).toEqual(before);
    expect(await call('batchCount')).toBe(count);
    await call('setVisibility', false);
    await expect(page.getByLabel('Answer draft')).toHaveValue('An unsent answer');
    await expect(page.getByLabel('Ordinary draft')).toHaveValue('An unsent proposal');
    await flushResize(page);
    expect(await call('proof')).toEqual(before);
    expect(await call('batchCount')).toBe(count);
  });

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const)
    test(`all-slot disclosures ${width} ${theme}: nested visibility and draft survival`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 1100 });
      await page.emulateMedia({ colorScheme: theme });
      await page.addInitScript((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      for (const kind of ['proposal', 'review', 'reading'] as const) {
        await page.goto('/');
        await page.evaluate(() => {
          const post = MessagePort.prototype.postMessage;
          Reflect.set(window, 'slotRequestCount', 0);
          MessagePort.prototype.postMessage = function (message, options) {
            if (message?.type === 'colab.render.slots')
              Reflect.set(
                window,
                'slotRequestCount',
                Number(Reflect.get(window, 'slotRequestCount')) + 1,
              );
            return Reflect.apply(post, this, [message, options]);
          };
        });
        const call = (method: string, argument?: unknown) =>
          fixtureCall(page, 'ask-page-browser', method, argument);
        await call('mount', { statusAgents: true });
        await call('detailsSlots', { kind, darkPage: theme === 'dark' });
        const frame = page.frameLocator('#ask-page-fixture iframe');
        const outer = frame.locator('#outer'),
          inner = frame.locator('#inner');
        const slot = page.locator(`[data-inline-${kind}]`).first();
        await expect(slot).toHaveCount(1);
        const before = await call('discussionProof');
        const hidden = async () => {
          await expect(slot).toHaveCSS('display', 'none');
          const button = slot.locator('button').first();
          expect(
            await button.evaluate((node) => {
              (node as HTMLElement).focus();
              return document.activeElement === node;
            }),
          ).toBe(false);
        };
        const shot = async (state: string) => {
          const name = `slots-${kind}-${state}-${width}-${theme}.png`;
          const body = await page.screenshot({ path: capturePath(name) });
          await test.info().attach(name, { body, contentType: 'image/png' });
        };
        await hidden();
        await shot('closed');
        await outer.locator(':scope > summary').click();
        await expect(slot).toBeVisible();
        await expect(slot).not.toHaveAttribute('data-detached');
        let draft;
        if (kind === 'review') {
          await slot.getByRole('button', { name: 'Add a note', exact: true }).click();
          draft = slot.getByLabel('Note (optional)', { exact: true });
          await draft.fill('Keep this review draft');
        } else if (kind === 'proposal') {
          await slot.getByRole('button', { name: 'Follow up', exact: true }).click();
          draft = slot.getByRole('combobox');
          await draft.fill('Keep this proposal draft');
        }
        await expect
          .poll(async () => {
            const box = await slot.boundingBox();
            const tag = kind === 'reading' ? 'read' : kind;
            const spacer =
              kind === 'reading'
                ? frame.locator('tmt-read > [data-colab-proposal-slot]')
                : frame.locator(`tmt-${tag} + [data-colab-proposal-slot]`);
            const reserved = await spacer.boundingBox();
            return (
              !!box &&
              !!reserved &&
              Math.abs(box.y - reserved.y) <= 1 &&
              Math.abs(box.height - reserved.height) <= 1
            );
          })
          .toBe(true);
        await shot('open');
        await inner.locator(':scope > summary').click();
        await hidden();
        await flushResize(page);
        const requests = await page.evaluate(() => Number(Reflect.get(window, 'slotRequestCount')));
        // An absence check spans three renderer debounce periods; navigation must
        // not drive a hide -> measure -> request -> hidden-report cycle.
        await expect(
          page.waitForFunction(
            (count) => Number(Reflect.get(window, 'slotRequestCount')) > count,
            requests,
            { timeout: 350, polling: 'raf' },
          ),
        ).rejects.toThrow(/Timeout/);
        await outer.locator(':scope > summary').click();
        await outer.locator(':scope > summary').click();
        await hidden();
        await inner.locator(':scope > summary').click();
        await expect(slot).toBeVisible();
        if (draft) {
          if (kind === 'proposal') await expect(draft).toHaveText('Keep this proposal draft');
          else await expect(draft).toHaveValue('Keep this review draft');
        }
        await outer.evaluate((node) => {
          (node as HTMLDetailsElement).open = false;
        });
        await hidden();
        await outer.evaluate((node) => {
          (node as HTMLDetailsElement).open = true;
        });
        await expect(slot).toBeVisible();
        if (draft) {
          if (kind === 'proposal') await expect(draft).toHaveText('Keep this proposal draft');
          else await expect(draft).toHaveValue('Keep this review draft');
        }
        expect(await call('discussionProof')).toEqual(before);
        expect((await call('proof')).sends).toHaveLength(0);
        expect((await call('proof')).actions).toEqual([]);
      }
    });

test('hidden questions stay out of header navigation and remain available in metadata', async ({
  page,
}) => {
  await page.goto('/');
  const call = (method: string, argument?: unknown) =>
    fixtureCall(page, 'ask-page-browser', method, argument);
  await call('mount');
  await call('detailsQuestions', { count: 1 });
  const slot = page.locator('.question-compact-inline');
  const line = slot.locator('.question-line');
  await expect(slot).toHaveCSS('display', 'none');
  await expect(page.locator('.needs-you-header-trigger')).toHaveCount(0);
  await (await pageAction(page, 'Questions…')).click();
  await expect(page.getByRole('dialog', { name: 'Questions', exact: true })).toBeVisible();
  await expect(line).toHaveAttribute('aria-expanded', 'false');
  await expect(slot).toHaveCSS('display', 'none');
  expect((await call('proof')).sends).toHaveLength(0);
  expect((await call('proof')).actions).toEqual([]);
});
