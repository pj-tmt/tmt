import { expect, test, type Page } from '@playwright/test';
import { text } from '../src/strings.js';
import { annotationInput } from '../acceptance/harness/ask.js';

const fixture = '/test/ask-page-browser.tsx';
const source = '<p id="selected">Exact selected text</p>';
const storageKey = 'colab-fixture-drafts:00000000-0000-4000-8000-000000000001';

type Mode = 'session' | 'failing';
async function mount(page: Page, drafts: Mode) {
  await page.evaluate(async ({ fixture, drafts }) => (await import(fixture)).mount({ drafts }), {
    fixture,
    drafts,
  });
  await expect(page.locator('#ask-page-fixture .status')).toContainText('Live preview');
}
async function call(page: Page, method: string, argument?: unknown) {
  return page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
}
async function select(page: Page) {
  await page
    .frameLocator('#ask-page-fixture iframe')
    .locator('#selected')
    .evaluate((node) => {
      const range = node.ownerDocument.createRange();
      range.selectNodeContents(node);
      const selection = node.ownerDocument.defaultView!.getSelection()!;
      selection.removeAllRanges();
      selection.addRange(range);
    });
  await page.getByTestId('selection-ask').click();
}
async function panel(page: Page, name: 'chat' | 'comments') {
  const host = page.locator('#ask-page-fixture');
  const toggle = host.getByTestId(`${name}-toggle`);
  if (!(await toggle.isVisible()))
    await host.getByRole('button', { name: 'More page actions' }).click();
  await toggle.click();
}
/** The fixture store has accepted a draft once sessionStorage holds it: the durable signal. */
async function stored(page: Page, needle: string) {
  await expect
    .poll(() => page.evaluate((key) => sessionStorage.getItem(key) ?? '', storageKey))
    .toContain(needle);
}
async function reload(page: Page, drafts: Mode) {
  await page.reload();
  await call(page, 'change', source);
  await mount(page, drafts);
}

for (const width of [1440, 390]) {
  test(`a selection draft survives reload and returns when the quote is selected again at ${width}`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.goto('/');
    await mount(page, 'session');
    await select(page);
    const composer = page.getByRole('dialog', { name: 'Annotate selection' });
    const input = await annotationInput(composer, 'Agent 1');
    const draft = '@Agent 1 Multiline\n文 draft with 🐈 that reload must keep.';
    await input.fill(draft);
    await page.getByRole('button', { name: 'Close annotation', exact: true }).click();
    await stored(page, 'Multiline');

    await reload(page, 'session');
    await panel(page, 'comments');
    const saved = page.getByTestId('saved-draft');
    await expect(saved).toHaveCount(1);
    await expect(saved.locator('blockquote')).toHaveText('Exact selected text');
    await expect(saved.locator('.saved-draft-text')).toHaveText(draft);
    await expect(saved.getByText(text.savedDraftQuote)).toBeVisible();
    await page.getByRole('button', { name: 'Close Comments', exact: true }).click();

    await select(page);
    await expect(composer.getByText('Draft kept', { exact: true })).toBeVisible();
    await expect(input).toHaveText(draft, { useInnerText: true });
    // Restoring neither prepares nor sends.
    expect((await call(page, 'proof')).sends).toEqual([]);
  });
}

test('Chat and thread reply drafts survive reload; an emptied draft stays gone', async ({
  page,
}) => {
  await page.goto('/');
  await mount(page, 'session');
  await panel(page, 'chat');
  const chat = await annotationInput(page.getByTestId('chat-panel'), 'Agent 1');
  await chat.fill('Chat draft that survives.');
  await stored(page, 'Chat draft that survives.');
  await reload(page, 'session');
  await panel(page, 'chat');
  const restored = await annotationInput(page.getByTestId('chat-panel'), 'Agent 1');
  await expect(restored).toHaveText('Chat draft that survives.', { useInnerText: true });
  await restored.press('ControlOrMeta+A');
  await restored.press('Backspace');
  await expect
    .poll(() => page.evaluate((key) => sessionStorage.getItem(key) ?? '', storageKey))
    .not.toContain('Chat draft');
  await reload(page, 'session');
  await panel(page, 'chat');
  await expect(await annotationInput(page.getByTestId('chat-panel'), 'Agent 1')).toHaveText('', {
    useInnerText: true,
  });
  expect((await call(page, 'proof')).sends).toEqual([]);
});

test('a draft for a thread that no longer exists is listed, readable and discardable', async ({
  page,
}) => {
  await page.goto('/');
  const gone = '00000000-0000-4000-8000-000000000004:00000000-0000-4000-8000-0000000000aa';
  await page.evaluate(
    ({ key, gone }) =>
      sessionStorage.setItem(
        key,
        JSON.stringify([[gone, { value: 'Reply to a vanished thread.' }]]),
      ),
    { key: storageKey, gone },
  );
  await mount(page, 'session');
  await panel(page, 'comments');
  const saved = page.getByTestId('saved-draft');
  await expect(saved).toHaveCount(1);
  await expect(saved.locator('.saved-draft-text')).toHaveText('Reply to a vanished thread.');
  await expect(saved.getByText(text.savedDraftThreadGone)).toBeVisible();
  await saved.getByRole('button', { name: text.savedDraftDiscard }).click();
  await expect(saved).toHaveCount(0);
  await expect
    .poll(() => page.evaluate((key) => sessionStorage.getItem(key) ?? '', storageKey))
    .not.toContain('vanished');
});

test('failed device storage says drafts are not saved and keeps the in-tab draft', async ({
  page,
}) => {
  await page.goto('/');
  await mount(page, 'failing');
  await select(page);
  const composer = page.getByRole('dialog', { name: 'Annotate selection' });
  const input = await annotationInput(composer, 'Agent 1');
  await input.fill('@Agent 1 Only in this tab.');
  await expect(composer.getByText(text.draftsNotSaved)).toBeVisible();
  await expect(input).toHaveText('@Agent 1 Only in this tab.', { useInnerText: true });
  await page.getByRole('button', { name: 'Close annotation', exact: true }).click();
  await select(page);
  await expect(input).toHaveText('@Agent 1 Only in this tab.', { useInnerText: true });
  expect((await call(page, 'proof')).sends).toEqual([]);
});

test('the real encrypted store keeps drafts in IndexedDB across reload and refuses a copied scope', async ({
  page,
}) => {
  await page.goto('/');
  const load = (space: string) =>
    page.evaluate(async (space) => {
      const path = '/src/draft-store.ts';
      const { LocalDraftStore } = await import(path);
      const loaded = await new LocalDraftStore(space, 'device').load('page');
      return loaded ? [...loaded] : null;
    }, space);
  const saved = await page.evaluate(async () => {
    const path = '/src/draft-store.ts';
    const { LocalDraftStore } = await import(path);
    const store = new LocalDraftStore('space', 'device');
    return store.apply(
      'page',
      new Map([['chat', { value: 'Real IndexedDB draft 🐈\nsecond line', edited: true }]]),
    );
  });
  expect(saved).toBe(true);
  await page.reload();
  expect(await load('space')).toEqual([
    ['chat', { value: 'Real IndexedDB draft 🐈\nsecond line', edited: true }],
  ]);
  // The same bytes under another scope do not decrypt.
  expect(await load('other-space')).toEqual([]);
  const raw = await page.evaluate(
    () =>
      new Promise<string>((resolve, reject) => {
        const open = indexedDB.open('tmt-colab', 1);
        open.onerror = () => reject(open.error);
        open.onsuccess = () => {
          const get = open.result
            .transaction('keys')
            .objectStore('keys')
            .get('drafts:space:device:page');
          get.onsuccess = () => resolve(JSON.stringify(get.result));
        };
      }),
  );
  expect(raw).not.toContain('Real IndexedDB');
});
