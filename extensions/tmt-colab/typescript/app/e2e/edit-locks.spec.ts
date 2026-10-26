import { readFileSync } from 'node:fs';
import { expect, test, type Page } from '@playwright/test';
const fixture = '/test/ask-page-browser.tsx';
const errors = new WeakMap<Page, string[]>();
test.beforeEach(async ({ page }) => {
  // The isolated fixture has no Remote door; resolve its optional shell discovery explicitly.
  await page.route('**/sdk/mount', (route) => route.fulfill({ status: 204 }));
  const found: string[] = [];
  errors.set(page, found);
  page.on('pageerror', (error) => found.push(error.message));
  page.on('console', (message) => {
    if (message.type() === 'error') found.push(message.text());
  });
});
test.afterEach(({ page }) => {
  expect(errors.get(page)).toEqual([]);
});
async function rendererFrame(page: Page, root = '#ask-page-fixture') {
  const element = await page.locator(`${root} .frame-host iframe`).elementHandle();
  const frame = await element?.contentFrame();
  await element?.dispose();
  if (!frame) throw new Error('Missing fixture page renderer');
  return frame;
}
const record = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/edit-lock-v1.json', import.meta.url), 'utf8'),
).record;
async function run(page: Page, method: string, argument?: unknown) {
  return page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
}
async function open(page: Page) {
  await page.goto('/test/edit-locks-browser.html');
  await run(page, 'mount');
  await expect(page.locator('#ask-page-fixture .status')).toContainText('Live');
  const source =
    '<style>p{line-height:2}body>p:last-child{padding-top:111px}</style>' +
    Array.from(
      { length: 70 },
      (_, n) => `<p id="block-${n}">Passage ${n} stays in place.</p>`,
    ).join('\n');
  await run(page, 'authored', source);
  await expect(
    page.frameLocator('#ask-page-fixture .frame-host iframe').locator('#block-69'),
  ).toBeAttached();
  await expect
    .poll(() => page.evaluate(() => document.documentElement.scrollHeight))
    .toBeGreaterThan(2000);
  await page.evaluate(() => window.scrollTo(0, 300));
  return source;
}
async function geometry(page: Page) {
  const frame = await rendererFrame(page);
  return {
    page: await page.evaluate(() => ({
      scroll: scrollY,
      height: document.documentElement.scrollHeight,
      chrome: [...document.querySelectorAll('#ask-page-fixture header, #ask-page-fixture nav')].map(
        (node) => {
          const box = node.getBoundingClientRect();
          return [box.left, box.top, box.width, box.height, node.textContent];
        },
      ),
    })),
    frame: await frame.evaluate(() => ({
      scroll: scrollY,
      height: document.documentElement.scrollHeight,
      children: document.body.childNodes.length,
      blocks: [...document.querySelectorAll('p')].map((node) => {
        const box = node.getBoundingClientRect();
        return [box.left, box.top + scrollY, box.width, box.height];
      }),
    })),
  };
}
test('whole-page edit locks float without moving blocks, changing scroll or adding page nodes', async ({
  page,
}, info) => {
  await open(page);
  const before = await geometry(page);
  expect(before.frame.blocks).toHaveLength(70);
  await page.screenshot({ path: info.outputPath('whole-before.png') });
  await run(page, 'setEditLocks', [{ ...record, target: { whole: true } }]);
  const chip = page.getByTestId('edit-lock');
  await expect(chip).toHaveText('night-5 is editing');
  await expect(chip).toHaveAttribute('data-anchor', 'page');
  expect(await geometry(page)).toEqual(before);
  await page.screenshot({ path: info.outputPath('whole-lock.png') });
  await run(page, 'setEditLocks', [
    { ...record, target: { whole: true }, revision: '2', released: true },
  ]);
  await expect(chip).toHaveCount(0);
  expect(await geometry(page)).toEqual(before);
});
test('a source quote paints only its unambiguous DOM range and clears without layout shift', async ({
  page,
}, info) => {
  const source = await open(page);
  const exact = 'Passage 8 stays in place.';
  const at = source.indexOf(exact);
  const target = {
    exact,
    prefix: source.slice(Math.max(0, at - 32), at),
    suffix: source.slice(at + exact.length, at + exact.length + 32),
  };
  const before = await geometry(page);
  expect(before.frame.blocks).toHaveLength(70);
  await page.screenshot({ path: info.outputPath('range-before.png') });
  await run(page, 'setEditLocks', [{ ...record, target }]);
  const chip = page.getByTestId('edit-lock');
  await expect(chip).toHaveText('night-5 is editing');
  await expect(chip).toHaveAttribute('data-anchor', 'range');
  const frame = await rendererFrame(page);
  await expect
    .poll(() =>
      frame.evaluate(() => {
        const tint = CSS.highlights.get('colab-edit-locks');
        return tint ? [...tint].map((range) => range.toString()) : [];
      }),
    )
    .toEqual([exact]);
  expect(await geometry(page)).toEqual(before);
  await page.screenshot({ path: info.outputPath('range-lock.png') });
  await run(page, 'setEditLocks', []);
  await expect(chip).toHaveCount(0);
  await expect.poll(() => frame.evaluate(() => CSS.highlights.has('colab-edit-locks'))).toBe(false);
  expect(await geometry(page)).toEqual(before);
});
test('a missing source range uses the floating chip and never tints another passage', async ({
  page,
}) => {
  await open(page);
  await run(page, 'setEditLocks', [
    { ...record, target: { exact: 'missing passage', prefix: '', suffix: '' } },
  ]);
  await expect(page.getByTestId('edit-lock')).toHaveAttribute('data-anchor', 'page');
  const frame = await rendererFrame(page);
  expect(await frame.evaluate(() => CSS.highlights.has('colab-edit-locks'))).toBe(false);
});

test('markup and entities map to the actual text without inserting author nodes', async ({
  page,
}) => {
  await open(page);
  await run(page, 'authored', '<p id="mapped">Alpha &amp; <strong>Beta</strong> line.</p>');
  const frame = await rendererFrame(page);
  await expect(frame.locator('#mapped')).toHaveText('Alpha & Beta line.');
  const before = await frame.evaluate(() => document.body.childNodes.length);
  await run(page, 'setEditLocks', [
    {
      ...record,
      target: { exact: '&amp; <strong>Beta</strong>', prefix: 'Alpha ', suffix: ' line.' },
    },
  ]);
  await expect(page.getByTestId('edit-lock')).toHaveAttribute('data-anchor', 'range');
  expect(
    await frame.evaluate(() =>
      [...(CSS.highlights.get('colab-edit-locks') ?? [])].map((range) => range.toString()),
    ),
  ).toEqual(['& Beta']);
  expect(await frame.evaluate(() => document.body.childNodes.length)).toBe(before);
});
test('an attribute source match cannot tint an unrelated decoded text impostor', async ({
  page,
}) => {
  await open(page);
  await run(
    page,
    'authored',
    '<p title="Unique phrase">A labelled paragraph</p><p id="impostor">&#85;nique phrase</p>',
  );
  const frame = await rendererFrame(page);
  await expect(frame.locator('#impostor')).toHaveText('Unique phrase');
  await run(page, 'setEditLocks', [
    { ...record, target: { exact: 'Unique phrase', prefix: '', suffix: '' } },
  ]);
  await expect(page.getByTestId('edit-lock')).toHaveAttribute('data-anchor', 'page');
  expect(await frame.evaluate(() => CSS.highlights.has('colab-edit-locks'))).toBe(false);
});

test('ambiguous normalized source paragraphs use the floating chip', async ({ page }) => {
  await open(page);
  const content = 'Before ' + 'x'.repeat(40) + 'Target' + 'y'.repeat(40) + ' After';
  const paragraphs = Array.from({ length: 5 }, (_, i) => `<p id="repeat-${i}">${content}</p>`);
  await run(page, 'authored', paragraphs.join('\n'));
  const frame = await rendererFrame(page);
  await expect(frame.locator('#repeat-4')).toBeAttached();
  await run(page, 'setEditLocks', [
    { ...record, target: { exact: paragraphs[2], prefix: '', suffix: '' } },
  ]);
  await expect(page.getByTestId('edit-lock')).toHaveAttribute('data-anchor', 'page');
  expect(await frame.evaluate(() => CSS.highlights.has('colab-edit-locks'))).toBe(false);
});
test('fresh offline or active-ended observations hide locks while unknown and failed reads keep them', async ({
  page,
}) => {
  await open(page);
  let revision = 1;
  const holder = (presence: 'active' | 'offline' | 'unknown', ended = false) => ({
    agent: record.holder.id,
    presence,
    activity: { state: ended ? 'ended' : 'idle', sinceMs: null },
  });
  const cases = [
    { holders: [holder('offline')], count: 0 },
    { holders: [holder('unknown', true)], count: 1 },
    { holders: [holder('active', true)], count: 0 },
    { holders: [], count: 1 },
    { holders: [holder('offline')], count: 0 },
    { holders: [holder('offline')], failed: true, count: 1 },
    { holders: [holder('active')], count: 1 },
  ];
  for (const { count, ...observation } of cases) {
    const before = await run(page, 'editLockDirectoryReads');
    await run(page, 'setEditLockHolders', observation);
    await run(page, 'setEditLocks', [
      { ...record, target: { whole: true }, revision: String(revision++) },
    ]);
    await expect.poll(() => run(page, 'editLockDirectoryReads')).toBeGreaterThan(before);
    await page.evaluate(
      () =>
        new Promise<void>((resolve) =>
          requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
        ),
    );
    await expect(page.getByTestId('edit-lock')).toHaveCount(count);
  }
});

test('range chips follow internal frame scrolling without scrolling the parent page', async ({
  page,
}) => {
  await page.goto('/test/edit-locks-browser.html');
  await run(page, 'mount');
  await expect(page.locator('#ask-page-fixture .status')).toContainText('Live');
  await run(
    page,
    'authored',
    '<style>body{margin:0}.grow{height:calc(100vh + 500px)}</style><div class="grow"><p id="scroll-target" style="padding-top:200px;margin:0">Scroll-locked text</p></div>',
  );
  const iframe = page.locator('#ask-page-fixture .frame-host iframe');
  await expect(iframe).toHaveAttribute('data-scroll-mode', 'frame');
  const frame = await rendererFrame(page);
  await run(page, 'setEditLocks', [
    { ...record, target: { exact: 'Scroll-locked text', prefix: '', suffix: '' } },
  ]);
  const chip = page.getByTestId('edit-lock');
  await expect(chip).toHaveAttribute('data-anchor', 'range');
  const before = (await chip.boundingBox())!.y;
  const parentScroll = await page.evaluate(() => scrollY);
  await frame.evaluate(() => scrollTo(0, 100));
  await expect
    .poll(async () => Math.round((await chip.boundingBox())!.y))
    .toBe(Math.round(before - 100));
  expect(await page.evaluate(() => scrollY)).toBe(parentScroll);
  await frame.evaluate(() => scrollTo(0, 400));
  await expect(chip).toHaveCount(0);
  await frame.evaluate(() => scrollTo(0, 0));
  await expect(chip).toBeVisible();
  expect(await page.evaluate(() => scrollY)).toBe(parentScroll);
});

test.describe('mobile dark', () => {
  test.use({ viewport: { width: 390, height: 844 }, colorScheme: 'dark' });
  test('range and whole-page identity chips stay legible and preserve geometry', async ({
    page,
  }, info) => {
    const source = await open(page);
    const exact = 'Passage 8 stays in place.',
      at = source.indexOf(exact);
    const target = {
      exact,
      prefix: source.slice(at - 32, at),
      suffix: source.slice(at + exact.length, at + exact.length + 32),
    };
    const before = await geometry(page);
    expect(before.frame.blocks).toHaveLength(70);
    await page.screenshot({ path: info.outputPath('mobile-dark-before.png') });
    await run(page, 'setEditLocks', [
      { ...record, target: { whole: true } },
      {
        ...record,
        lockId: '40000000-0000-4000-8000-000000000002',
        holder: { id: '50000000-0000-7000-8000-000000000002', name: 'Another agent' },
        target,
      },
    ]);
    await expect(page.getByTestId('edit-lock')).toHaveCount(2);
    await expect(
      page.getByTestId('edit-lock').filter({ hasText: 'Another agent' }),
    ).toHaveAttribute('data-anchor', 'range');
    expect(await geometry(page)).toEqual(before);
    await page.screenshot({ path: info.outputPath('mobile-dark-locks.png') });
  });
});

test('shared readers display verified lock hints without the owner stylesheet or content insertion', async ({
  page,
}) => {
  await page.goto('/test/edit-locks-browser.html');
  const source = '<p id="reader-target">Shared passage</p>';
  await page.evaluate(
    async ({ source, record }) => {
      const path = '/test/page-layout-browser.tsx';
      const fixture = await import(path);
      await fixture.mountReader(source, {
        editLocks: [{ ...record, target: { exact: 'Shared passage', prefix: '', suffix: '' } }],
      });
      document
        .querySelectorAll('style[data-vite-dev-id$="/src/style.css"]')
        .forEach((node) => node.remove());
    },
    { source, record },
  );
  const frame = await rendererFrame(page, '#layout-fixture');
  await expect(frame.locator('#reader-target')).toHaveText('Shared passage');
  await expect(page.getByTestId('edit-lock')).toHaveAttribute('data-anchor', 'range');
  expect(
    await frame.evaluate(() =>
      [...(CSS.highlights.get('colab-edit-locks') ?? [])].map((range) => range.toString()),
    ),
  ).toEqual(['Shared passage']);
  expect(
    await frame.evaluate(() => document.querySelectorAll('[data-testid="edit-lock"]').length),
  ).toBe(0);
  await expect(page.getByTestId('edit-lock')).toHaveCSS('position', 'absolute');
  await page.screenshot({ path: test.info().outputPath('reader-lock.png') });
});
