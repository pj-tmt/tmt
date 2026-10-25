import { expect, test, type Frame, type Page } from '@playwright/test';

/**
 * A source change reaches the open frame in place when the page holds no script (#5022): the same
 * document keeps its own state (a frame-local value, inner scroll, a typed value, an opened
 * details), and the anchors and slots drawn over it are placed again. A script, an inline handler
 * or a layout change still gets a fresh frame, and so does a frame that refuses or does not answer.
 */
const fixture = '/test/ask-page-browser.tsx';
const run = (page: Page, method: string, argument?: unknown) =>
  page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
const iframe = (page: Page) => page.locator('#ask-page-fixture iframe');
const frame = (page: Page) => page.frameLocator('#ask-page-fixture iframe');
async function frameDocument(page: Page): Promise<Frame> {
  const handle = await iframe(page).elementHandle();
  return (await handle!.contentFrame())!;
}
const paragraphs = Array.from(
  { length: 150 },
  (_, index) =>
    `<p id="para-${index}">Paragraph ${index} has its own unique sentence, and enough ordinary words to fill a line.</p>`,
).join('\n');
const long = (options: { color?: string; inserted?: string; extra?: string } = {}) =>
  `<style>p{color:${options.color ?? 'rgb(1, 2, 3)'}}#scroller{height:80px;overflow:auto}</style>
<p id="intro">Intro <tmt-icon id="icon" name="check"></tmt-icon></p>
<div id="scroller">${Array.from({ length: 30 }, (_, index) => `<div>Row ${index}</div>`).join('')}</div>
<input id="field" value="start">
<details id="more"><summary>More</summary><p>Inside the details.</p></details>
${options.extra ?? ''}${paragraphs.replace('<p id="para-1">', `${options.inserted ?? ''}<p id="para-1">`)}`;

/** A long page with three threads (paragraphs 0 to 2), open and ready, with a frame-local mark. */
async function open(page: Page) {
  await page.goto('/');
  await run(page, 'mount', { statusAgents: true });
  await run(page, 'largePage', { paragraphs: 3, threads: 3 });
  await run(page, 'change', long());
  await expect(frame(page).locator('#field')).toBeVisible();
  await expect(frame(page).locator('[data-colab-thread]')).toHaveCount(3);
  await (await frameDocument(page)).evaluate(() => Object.assign(window, { sameDocument: true }));
  return iframe(page).getAttribute('data-render-id');
}
const sameDocument = async (page: Page) =>
  (await frameDocument(page)).evaluate(
    () => (window as unknown as { sameDocument?: boolean }).sameDocument === true,
  );

test('a one-line edit of a script-free page keeps the frame, its state and its overlays', async ({
  page,
}) => {
  const renderId = await open(page);
  const doc = await frameDocument(page);
  await doc.locator('#scroller').evaluate((node) => (node.scrollTop = 200));
  await doc.locator('#field').fill('typed by the reader');
  await doc.locator('#more summary').click();
  await expect(doc.locator('#more')).toHaveAttribute('open', '');
  await expect(doc.locator('#icon svg')).toHaveCount(1);
  const marker = frame(page).locator('[data-colab-thread]').nth(1);
  const before = (await marker.boundingBox())!.y;

  await run(
    page,
    'change',
    long({ inserted: '<p id="inserted">A new line next to a passage.</p>' }),
  );
  await expect(frame(page).locator('#inserted')).toBeVisible();
  await expect(iframe(page)).toHaveAttribute('data-render-id', renderId!);
  expect(await sameDocument(page)).toBe(true);
  expect(await doc.locator('#scroller').evaluate((node) => node.scrollTop)).toBe(200);
  await expect(doc.locator('#field')).toHaveValue('typed by the reader');
  await expect(doc.locator('#more')).toHaveAttribute('open', '');
  // The icon the frame drew stays drawn, and the highlights come back, placed below the new line.
  await expect(doc.locator('#icon svg')).toHaveCount(1);
  await expect(frame(page).locator('[data-colab-thread]')).toHaveCount(3);
  await expect.poll(async () => (await marker.boundingBox())!.y).toBeGreaterThan(before + 10);
  await expect(page.locator('#ask-page-fixture .status')).toContainText('Live');

  // A style-only patch: the author's <style> lives in the body and changes in place.
  await run(page, 'change', long({ color: 'rgb(200, 0, 0)' }));
  await expect(doc.locator('#para-5')).toHaveCSS('color', 'rgb(200, 0, 0)');
  await expect(iframe(page)).toHaveAttribute('data-render-id', renderId!);
  expect(await sameDocument(page)).toBe(true);
});

test('an edit next to a proposal places its slot again in the same frame', async ({ page }) => {
  await page.goto('/');
  await run(page, 'mount');
  await run(page, 'proposal', 'inline');
  const card = page.locator('.proposal-inline').first();
  await expect(card).toBeVisible();
  const renderId = await iframe(page).getAttribute('data-render-id');
  const before = (await card.boundingBox())!.y;
  const source = (await run(page, 'currentSource')) as string;
  await run(
    page,
    'change',
    source.replace('<p>Before the proposal.</p>', '$&<div style="height:300px">Tall</div>'),
  );
  await expect(frame(page).getByText('Tall', { exact: true })).toBeVisible();
  await expect(iframe(page)).toHaveAttribute('data-render-id', renderId!);
  await expect.poll(async () => (await card.boundingBox())!.y).toBeGreaterThan(before + 250);
});

for (const [name, extra] of [
  ['a script', '<script>window.ran = true;</script>'],
  ['an inline handler', '<button onclick="1">Press</button>'],
]) {
  test(`adding ${name} gets a fresh frame`, async ({ page }) => {
    const renderId = await open(page);
    await run(page, 'change', long({ extra }));
    await expect(iframe(page)).not.toHaveAttribute('data-render-id', renderId!);
    await expect(frame(page).locator('#field')).toBeVisible();
    expect(await sameDocument(page)).toBe(false);
  });
}

test('a layout change gets a fresh frame', async ({ page }) => {
  const renderId = await open(page);
  await run(page, 'setLayout', { layout: 'column' });
  await expect(iframe(page)).not.toHaveAttribute('data-render-id', renderId!);
  await expect(frame(page).locator('#field')).toBeVisible();
  expect(await sameDocument(page)).toBe(false);
});

for (const [name, replacement] of [
  // The frame answers that the page needs a fresh frame.
  ['refuses', 'applied = false;'],
  // The frame never answers: the parent's deadline mounts afresh.
  ['never answers', 'return;'],
]) {
  test(`a frame that ${name} is replaced by a fresh one with the new source`, async ({ page }) => {
    await page.route('**/renderer.html', async (route) => {
      const response = await route.fetch();
      const body = await response.text();
      const call = 'applied = applySource(v.source);';
      if (!body.includes(call)) throw new Error('The renderer no longer applies sources there');
      await route.fulfill({ response, body: body.replace(call, replacement) });
    });
    const renderId = await open(page);
    await run(page, 'change', long({ inserted: '<p id="inserted">After the fallback.</p>' }));
    await expect(iframe(page)).not.toHaveAttribute('data-render-id', renderId!, {
      timeout: 5000,
    });
    await expect(frame(page).locator('#inserted')).toBeVisible();
    expect(await sameDocument(page)).toBe(false);
  });
}

test('a newer update supersedes an older one, and an oversized source remounts', async ({
  page,
}) => {
  await page.goto('/');
  const results = await page.evaluate(async () => {
    const path = '/src/renderer.ts';
    const { mountRenderer } = await import(path);
    const host = document.createElement('div');
    document.body.replaceChildren(host);
    let ready!: () => void;
    const bound = new Promise<void>((resolve) => (ready = resolve));
    const handle = await mountRenderer(host, '<p id="a">one</p>', {
      signal: new AbortController().signal,
      onState: (state: string) => state === 'ready' && ready(),
    });
    await bound;
    const [older, newer] = await Promise.all([
      handle.update('<p id="a">two</p>'),
      handle.update('<p id="a">three</p>'),
    ]);
    const shown = host.querySelector('iframe')!.dataset.sourceDigest;
    const oversized = await handle.update('x'.repeat(2 * 1024 * 1024 + 1));
    return {
      older,
      newer,
      oversized,
      shown,
      renderId: host.querySelector('iframe')!.dataset.renderId,
    };
  });
  expect(results).toMatchObject({ older: 'superseded', newer: 'applied', oversized: 'remount' });
  // The binding identity stays; the shown source's digest is tracked on its own.
  expect(results.renderId!.endsWith(`:${results.shown}`)).toBe(false);
  await expect(page.frameLocator('iframe').locator('#a')).toHaveText('three');
});
