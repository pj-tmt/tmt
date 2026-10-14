import { expect, test, type Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';
import { pageAction } from '../test/page-actions.js';
import { destination, id } from '../test/ask-fixtures.js';
import type { AgentDestination } from '../src/live-ask.js';
const fixture = '/test/annotation-browser.tsx';
async function run(page: Page, method: string, argument?: unknown) {
  return page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
}
const alpha = { ...destination(), agentName: 'alpha' };
async function mount(
  page: Page,
  surface = 'chat',
  options: {
    scope?: string;
    agents?: AgentDestination[];
    creator?: boolean;
    talk?: boolean;
    initialEdit?: unknown;
    directory?: 'park' | 'fail';
  } = {},
) {
  await page.goto('/');
  await run(page, 'setSurface', surface);
  await run(page, 'setDirectory', options.directory ?? 'ok');
  await run(page, 'setTalk', options.talk ?? true);
  await run(page, 'setAgents', options.agents ?? [alpha]);
  await run(page, 'setCreator', options.creator ?? true);
  await run(page, 'setPreferenceScope', options.scope ?? 'creator-default-proof');
  await run(page, 'setInitialEdit', options.initialEdit);
  await run(page, 'mount', 'accepted');
  const input = page.getByRole('combobox', { name: 'Message', exact: true }).first();
  await expect(input).toBeVisible();
  if (options.directory !== 'park')
    await expect(page.locator('.annotation-hint[role="status"]').first()).not.toContainText(
      'Checking',
    );
  return input;
}
const toggle = (page: Page) => page.getByRole('button', { name: 'Send to @alpha', exact: true });
for (const surface of ['annotation', 'reply', 'chat']) {
  test(`${surface}: every fresh message gets one admitted default, removed for one message only`, async ({
    page,
  }) => {
    const input = await mount(page, surface);
    await expect(input).toHaveText('@alpha ', { useInnerText: true });
    await input.pressSequentially('First');
    await input.press('Enter');
    await expect
      .poll(() => run(page, 'proof'))
      .toMatchObject({ writes: 1, preparations: 1, commits: 1 });
    await expect(input).toHaveText('@alpha ', { useInnerText: true });
    await page.getByRole('button', { name: /^Remove mention: @alpha/ }).click();
    await input.pressSequentially('Plain');
    if (surface === 'chat') {
      await expect(toggle(page)).toHaveCount(0);
      await expect(page.locator('.annotation-hint[role="status"]').first()).toContainText(
        'Posts as a comment.',
      );
    }
    await input.press('Enter');
    await expect
      .poll(() => run(page, 'proof'))
      .toMatchObject({ writes: 2, preparations: 1, commits: 2 });
    await expect(input).toHaveText('@alpha ', { useInnerText: true });
    if (surface === 'chat') {
      await expect(toggle(page)).toBeVisible();
      await expect(toggle(page)).toHaveAttribute('aria-pressed', 'true');
      await expect(page.locator('.annotation-hint[role="status"]').first()).not.toContainText(
        'Posts as a comment.',
      );
    }
    const proof = await run(page, 'editingProof');
    expect(
      proof.captured
        .filter((entry: { body?: string }) => entry.body !== undefined)
        .map((entry: { body: string }) => entry.body),
    ).toEqual(['@alpha First', 'Plain ']);
  });
}
test('toggle updates the open draft, preserves its body/caret/undo, and stays off for the next message', async ({
  page,
}) => {
  const input = await mount(page);
  await input.pressSequentially('Body line');
  await input.press('Shift+Enter');
  await input.pressSequentially('second');
  await input.press('ArrowLeft');
  const before = await input.evaluate(() => ({
    text: document.getSelection()?.anchorNode?.textContent,
    offset: document.getSelection()?.anchorOffset,
  }));
  await toggle(page).click();
  await expect(input).toHaveText(' Body line\nsecond', { useInnerText: true });
  expect(
    await input.evaluate(() => ({
      text: document.getSelection()?.anchorNode?.textContent,
      offset: document.getSelection()?.anchorOffset,
    })),
  ).toEqual(before);
  await input.press('ControlOrMeta+z');
  await expect(input).toHaveText('@alpha Body line\nsecond', { useInnerText: true });
  await input.press('ControlOrMeta+Shift+z');
  await expect(input).toHaveText(' Body line\nsecond', { useInnerText: true });
  await input.press('Enter');
  await expect
    .poll(() => run(page, 'proof'))
    .toMatchObject({ writes: 1, preparations: 0, commits: 1 });
  await expect(input).toHaveText('', { useInnerText: true });
  await expect(toggle(page)).toHaveAttribute('aria-pressed', 'false');
});
test('manual typed and picked mentions survive off; toggling on does not duplicate them', async ({
  page,
}) => {
  const input = await mount(page);
  await toggle(page).click();
  await input.fill('@alpha Manual');
  await expect(page.locator('.message-mention-chip')).toHaveCount(1);
  await toggle(page).click();
  await toggle(page).click();
  await expect(input).toHaveText('@alpha Manual', { useInnerText: true });
  await expect(page.locator('.message-mention-chip')).toHaveCount(1);
  await input.fill('@');
  await page.getByRole('option', { name: /@alpha/ }).click();
  await toggle(page).click();
  await toggle(page).click();
  await expect(page.locator('.message-mention-chip')).toHaveCount(1);
  await expect(input).toHaveText('@alpha ', { useInnerText: true });
  expect(await run(page, 'proof')).toMatchObject({ writes: 0, preparations: 0 });
});
test('page/viewer preference persists across reload and surfaces, with isolated scopes', async ({
  page,
}) => {
  await mount(page, 'chat');
  await toggle(page).click();
  let input = await mount(page, 'reply');
  await expect(toggle(page)).toHaveAttribute('aria-pressed', 'false');
  await expect(input).toHaveText('', { useInnerText: true });
  input = await mount(page, 'annotation', { scope: 'different-page' });
  await expect(toggle(page)).toHaveAttribute('aria-pressed', 'true');
  await expect(input).toHaveText('@alpha ', { useInnerText: true });
});
test('restored unknown-provenance creator mention is manual, and off does not delete it', async ({
  page,
}) => {
  const input = await mount(page, 'reply', {
    initialEdit: {
      value: '@alpha Saved',
      edited: true,
      mentions: [
        { key: { machine: alpha.machine, agent: alpha.agent }, range: { start: 0, end: 6 } },
      ],
    },
  });
  await toggle(page).click();
  await expect(input).toHaveText('@alpha Saved', { useInnerText: true });
  await expect(page.locator('.message-mention-chip')).toHaveCount(1);
});
test('an offline saved creator stays seeded, and an offline agent on an online machine receives', async ({
  page,
}) => {
  const unavailable = await mount(page, 'chat', {
    agents: [{ ...alpha, online: 'offline' }],
  });
  await expect(toggle(page)).toBeVisible();
  await expect(unavailable).toHaveText('@alpha ', { useInnerText: true });
  await unavailable.pressSequentially('Waiting');
  await expect(
    page.getByRole('button', { name: /^Send(?: · to .+)?$/, exact: true }),
  ).toBeDisabled();
  expect(await run(page, 'proof')).toMatchObject({ writes: 0, preparations: 0 });

  const input = await mount(page, 'chat', {
    agents: [{ ...alpha, presence: 'offline' }],
  });
  await expect(toggle(page)).toBeVisible();
  await expect(input).toHaveText('@alpha ', { useInnerText: true });
  await input.pressSequentially('Inbox');
  await input.press('Enter');
  await expect
    .poll(() => run(page, 'proof'))
    .toMatchObject({ writes: 1, preparations: 1, commits: 1 });
  await expect(input).toHaveText('@alpha ', { useInnerText: true });
});
test('no creator admission hides the toggle and never invents a fallback recipient', async ({
  page,
}) => {
  for (const agents of [
    [],
    [{ ...alpha, agent: id(9) }],
    [alpha, alpha],
    [alpha, { ...alpha, online: 'offline' as typeof alpha.online }],
  ]) {
    const input = await mount(page, 'chat', { agents });
    await expect(toggle(page)).toHaveCount(0);
    await expect(input).toHaveText('', { useInnerText: true });
    await input.pressSequentially('Comment');
    expect(await run(page, 'proof')).toMatchObject({ writes: 0, preparations: 0 });
  }
});
for (const variant of [
  { width: 1440, theme: 'light' },
  { width: 390, theme: 'dark' },
] as const) {
  test(`responsive creator action ${variant.width} ${variant.theme}`, async ({ page }) => {
    await page.setViewportSize({ width: variant.width, height: 900 });
    await page.emulateMedia({ colorScheme: variant.theme });
    await mount(page);
    await expect(toggle(page)).toHaveAttribute('aria-pressed', 'true');
    await expect(toggle(page)).toHaveText(variant.width <= 640 ? /@alpha/ : /Send to @alpha/);
    const geometry = await toggle(page).evaluate((node) => {
      const r = node.getBoundingClientRect();
      return { left: r.left, right: r.right };
    });
    expect(geometry.left).toBeGreaterThanOrEqual(0);
    expect(geometry.right).toBeLessThanOrEqual(variant.width);
    await expect(
      page.getByRole('button', { name: /^Send(?: · to .+)?$/, exact: true }),
    ).toBeVisible();
  });
}

test('storage refusal retains the tab-local off preference without changing Send admission', async ({
  page,
}) => {
  await page.addInitScript(() => {
    const get = Storage.prototype.getItem,
      set = Storage.prototype.setItem;
    Storage.prototype.getItem = function (key) {
      if (key.startsWith('tmt-colab-creator-default-v1:')) throw new Error('Refused');
      return get.call(this, key);
    };
    Storage.prototype.setItem = function (key, value) {
      if (key.startsWith('tmt-colab-creator-default-v1:')) throw new Error('Refused');
      return set.call(this, key, value);
    };
  });
  const input = await mount(page);
  await toggle(page).click();
  await expect(toggle(page)).toHaveAttribute('aria-pressed', 'false');
  await input.fill('Local only');
  await input.press('Enter');
  await expect.poll(() => run(page, 'proof')).toMatchObject({ writes: 1, preparations: 0 });
  await expect(input).toHaveText('', { useInnerText: true });
});
test('a preference changed during Send affects only the next draft, not the frozen message', async ({
  page,
}) => {
  const input = await mount(page);
  await run(page, 'holdWrite');
  await input.pressSequentially('Frozen');
  await page.getByRole('button', { name: /^Send(?: · to .+)?$/, exact: true }).click();
  await expect(input).toHaveAttribute('contenteditable', 'false');
  await toggle(page).click();
  await run(page, 'releaseWrite');
  await expect
    .poll(() => run(page, 'proof'))
    .toMatchObject({ writes: 1, preparations: 1, commits: 1 });
  await expect(input).toHaveText('', { useInnerText: true });
  const proof = await run(page, 'editingProof');
  expect(proof.captured[0]).toMatchObject({ body: '@alpha Frozen' });
  expect(proof.captured[1]).toMatchObject({ comment: '@alpha Frozen' });
});

for (const variant of [
  { width: 1440, theme: 'light' },
  { width: 390, theme: 'dark' },
] as const) {
  test(`page creator toggle captures ${variant.width} ${variant.theme}`, async ({ page }) => {
    const captures = process.env.COLAB_CREATOR_CAPTURE_DIR;
    test.skip(!captures, 'Set COLAB_CREATOR_CAPTURE_DIR for exact-head UX evidence.');
    mkdirSync(captures!, { recursive: true });
    await page.setViewportSize({ width: variant.width, height: 900 });
    await page.emulateMedia({ colorScheme: variant.theme });
    await page.addInitScript((theme) => {
      document.documentElement.dataset.theme = theme;
    }, variant.theme);
    async function open(creator: boolean) {
      await page.goto('/');
      await page.evaluate(
        async ({ creator, alpha }) => {
          const path = '/test/ask-page-browser.tsx';
          await (await import(path)).mount({ creator, agents: [alpha] });
        },
        { creator, alpha },
      );
      await page.evaluate((theme) => {
        document.documentElement.dataset.theme = theme;
      }, variant.theme);
      const app = page.locator('#ask-page-fixture');
      await (await pageAction(app, 'Chat')).click();
      return page.getByTestId('chat-panel').getByRole('combobox', { name: 'Message', exact: true });
    }
    const capture = async (state: string) =>
      page.screenshot({ path: `${captures}/${variant.width}-${variant.theme}-${state}.png` });
    const input = await open(true);
    await expect(toggle(page)).toHaveAttribute('aria-pressed', 'true');
    await expect(input.locator('.message-mention-chip')).toHaveCount(1);
    await capture('on');
    await toggle(page).click();
    await expect(input.locator('.message-mention-chip')).toHaveCount(0);
    await capture('off');
    await toggle(page).click();
    await input.getByRole('button', { name: /^Remove mention:/ }).click();
    await input.pressSequentially('A draft stays here.');
    // A manual comment has no creator toggle; the saved default returns on a fresh draft.
    await expect(toggle(page)).toHaveCount(0);
    await expect(
      page.getByText('Posts as a comment. No agent will be asked.', { exact: true }),
    ).toBeVisible();
    await capture('removed');
    await page.getByRole('button', { name: 'Send', exact: true }).click();
    await expect(toggle(page)).toHaveAttribute('aria-pressed', 'true');
    await expect(input.locator('.message-mention-chip')).toHaveCount(1);
    await capture('fresh');
    await open(false);
    await expect(toggle(page)).toHaveCount(0);
    await capture('hidden');
  });
}

test('a toggled seed alone is not a persisted draft; body edits are', async ({ page }) => {
  const input = await mount(page);
  await toggle(page).click();
  await toggle(page).click();
  await expect.poll(() => run(page, 'editingProof')).toMatchObject({ edit: { edited: false } });
  await input.pressSequentially('Keep this body');
  await expect.poll(() => run(page, 'editingProof')).toMatchObject({ edit: { edited: true } });
});
test('one page preference changes both mounted composers without publishing', async ({ page }) => {
  await mount(page, 'linked');
  const toggles = toggle(page);
  const chips = page.locator('.message-mention-chip');
  await expect(chips).toHaveCount(2);
  await toggles.first().click();
  await expect(toggles.nth(1)).toHaveAttribute('aria-pressed', 'false');
  await expect(chips).toHaveCount(0);
  await toggles.nth(1).click();
  await expect(toggles.first()).toHaveAttribute('aria-pressed', 'true');
  await expect(chips).toHaveCount(2);
  expect(await run(page, 'proof')).toMatchObject({ writes: 0, preparations: 0 });
});
test('disabled talk or missing creation identity hides the control, with no hidden recipient', async ({
  page,
}) => {
  for (const options of [{ talk: false }, { creator: false }]) {
    const input = await mount(page, 'chat', options);
    await expect(toggle(page)).toHaveCount(0);
    await expect(input.locator('.message-mention-chip')).toHaveCount(0);
    await input.fill('Plain comment');
    await input.press('Enter');
    await expect.poll(() => run(page, 'proof')).toMatchObject({ writes: 1, preparations: 0 });
  }
});

const commentOnly = 'Posts as a comment. No agent will be asked.';
for (const surface of ['chat', 'annotation']) {
  for (const selected of [false, true]) {
    test(`${surface}: a suppressed creator action preserves ${selected ? 'selection' : 'caret'} and only explicit Send asks`, async ({
      page,
    }) => {
      const input = await mount(page, surface, { directory: 'park' });
      await input.fill('Typed before the directory');
      await expect(page.getByRole('button', { name: 'Ask @alpha', exact: true })).toHaveCount(0);
      await expect(page.locator('.annotation-hint').first()).toHaveText('Checking for agents…');
      expect(await run(page, 'proof')).toMatchObject({ writes: 0, preparations: 0, commits: 0 });
      await run(page, 'releaseDirectory');
      const action = page.getByRole('button', { name: 'Ask @alpha', exact: true });
      await expect(action).toBeVisible();
      await expect(page.locator('.annotation-hint').first()).toHaveText(commentOnly);
      await expect(input).toHaveText('Typed before the directory');
      await input.evaluate((node, selected) => {
        const range = document.createRange();
        const text = document.createTreeWalker(node, NodeFilter.SHOW_TEXT).nextNode()!;
        range.setStart(text, 3);
        range.setEnd(text, selected ? 9 : 3);
        const selection = document.getSelection()!;
        selection.removeAllRanges();
        selection.addRange(range);
      }, selected);
      // Lexical can merge the inserted space into the body node; compare logical
      // text endpoints rather than that node's local offsets. Unit coverage pins UTF16 offsets.
      const before = await input.evaluate(() => ({
        anchorTail: document
          .getSelection()
          ?.anchorNode?.textContent?.slice(document.getSelection()!.anchorOffset),
        focusTail: document
          .getSelection()
          ?.focusNode?.textContent?.slice(document.getSelection()!.focusOffset),
        selected: document.getSelection()?.toString(),
      }));
      if (selected) {
        await action.focus();
        await action.press('Enter');
      } else await action.click();
      await expect(input).toBeFocused();
      await expect(input).toHaveText('@alpha Typed before the directory', { useInnerText: true });
      await expect(page.locator('.message-mention-chip')).toHaveCount(1);
      await expect(action).toHaveCount(0);
      await expect(page.locator('.annotation-hint').first()).toHaveText(
        surface === 'chat' ? '' : 'Asks @alpha.',
      );
      expect(
        await input.evaluate(() => ({
          anchorTail: document
            .getSelection()
            ?.anchorNode?.textContent?.slice(document.getSelection()!.anchorOffset),
          focusTail: document
            .getSelection()
            ?.focusNode?.textContent?.slice(document.getSelection()!.focusOffset),
          selected: document.getSelection()?.toString(),
        })),
      ).toEqual(before);
      expect(await run(page, 'proof')).toMatchObject({ writes: 0, preparations: 0, commits: 0 });
      await input.press('ControlOrMeta+z');
      await expect(input).toHaveText('Typed before the directory', { useInnerText: true });
      await input.press('ControlOrMeta+Shift+z');
      await expect(input).toHaveText('@alpha Typed before the directory', { useInnerText: true });
      await input.press('Enter');
      await expect
        .poll(() => run(page, 'proof'))
        .toMatchObject({ writes: 1, preparations: 1, commits: 1 });
      expect(
        (await run(page, 'editingProof')).captured
          .filter((item: { body?: string }) => item.body !== undefined)
          .map((item: { body: string }) => item.body),
      ).toEqual(['@alpha Typed before the directory']);
    });
  }
}
test('the comment-only hint needs exactly one creator and never replaces another mention state', async ({
  page,
}) => {
  for (const agents of [[], [alpha, alpha], [{ ...alpha, agent: id(9) }]]) {
    const input = await mount(page, 'chat', {
      agents,
      initialEdit: { value: 'Restored plain draft', edited: true },
    });
    await expect(page.getByRole('button', { name: 'Ask @alpha', exact: true })).toHaveCount(0);
    await expect(page.locator('.annotation-hint').first()).not.toHaveText(commentOnly);
    await expect(input).toHaveText('Restored plain draft');
  }
  await mount(page, 'chat', { initialEdit: { value: '@missing draft', edited: true } });
  await expect(page.getByRole('button', { name: 'Ask @alpha', exact: true })).toHaveCount(0);
  await expect(page.locator('.annotation-hint').first()).toContainText('No agent named @missing');
});
test('a removed creator offers an explicit manual choice without changing the saved default', async ({
  page,
}) => {
  const input = await mount(page);
  await page.getByRole('button', { name: /^Remove mention: @alpha/ }).click();
  await input.fill('A plain draft');
  await expect(page.locator('.annotation-hint').first()).toHaveText(commentOnly);
  await page.getByRole('button', { name: 'Ask @alpha', exact: true }).click();
  await expect(input).toHaveText('@alpha A plain draft', { useInnerText: true });
  await toggle(page).click();
  await expect(input).toHaveText('@alpha A plain draft', { useInnerText: true });
  expect(await run(page, 'proof')).toMatchObject({ writes: 0, preparations: 0, commits: 0 });
});

test('creator preference off keeps the one-off manual Ask action without changing the preference', async ({
  page,
}) => {
  const input = await mount(page);
  await toggle(page).click();
  await expect(toggle(page)).toHaveAttribute('aria-pressed', 'false');
  await input.fill('One-off request');
  await expect(toggle(page)).toHaveCount(0);
  await expect(page.locator('.annotation-hint').first()).toHaveText(commentOnly);
  await page.getByRole('button', { name: 'Ask @alpha', exact: true }).click();
  await expect(input).toHaveText('@alpha One-off request', { useInnerText: true });
  await expect(input).toBeFocused();
  await expect(toggle(page)).toHaveAttribute('aria-pressed', 'false');
  expect(await run(page, 'proof')).toMatchObject({ writes: 0, preparations: 0, commits: 0 });
});
