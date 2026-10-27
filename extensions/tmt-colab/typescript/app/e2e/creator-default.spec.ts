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
  if (options.creator !== false)
    await run(page, 'setCreatorMetadata', {
      agentId: alpha.agent,
      machineId: alpha.machine,
      label: 'alpha',
    });
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
// The creator preference is the last row of the recipient pill's agent list; Escape closes the list.
const pill = (page: Page) => page.getByRole('button', { name: /^Who is asked/ });
const creatorRow = (page: Page) =>
  page.getByRole('option', { name: /^Start new messages with @alpha/ });
async function toggle(page: Page, index = 0) {
  await pill(page).nth(index).click();
  await creatorRow(page).click();
}
async function creatorOn(page: Page, index = 0) {
  await pill(page).nth(index).click();
  const on = (await creatorRow(page).getAttribute('aria-selected')) === 'true';
  await page.keyboard.press('Escape');
  await expect(creatorRow(page)).toHaveCount(0);
  return on;
}
async function hasCreatorRow(page: Page) {
  if (!(await pill(page).first().isEnabled())) return false;
  await pill(page).first().click();
  await expect(page.getByRole('option').first()).toBeVisible();
  const has = (await creatorRow(page).count()) > 0;
  await page.keyboard.press('Escape');
  await expect(page.getByRole('listbox')).toHaveCount(0);
  return has;
}
/** The one-off manual choice: pick the agent in the same list. */
async function pickAlpha(page: Page) {
  await pill(page).first().click();
  await page.getByRole('option', { name: /^@alpha/ }).click();
}
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
      await expect(page.locator('.annotation-hint[role="status"]').first()).toContainText(
        'Comment only',
      );
    }
    await input.press('Enter');
    await expect
      .poll(() => run(page, 'proof'))
      .toMatchObject({ writes: 2, preparations: 1, commits: 2 });
    await expect(input).toHaveText('@alpha ', { useInnerText: true });
    if (surface === 'chat') {
      expect(await creatorOn(page)).toBe(true);
      await expect(page.locator('.annotation-hint[role="status"]').first()).not.toContainText(
        'Comment only',
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
  await toggle(page);
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
  expect(await creatorOn(page)).toBe(false);
});
test('manual typed and picked mentions survive off; toggling on does not duplicate them', async ({
  page,
}) => {
  const input = await mount(page);
  await toggle(page);
  await input.fill('@alpha Manual');
  await expect(page.locator('.agent-chip[data-variant="chip"]')).toHaveCount(1);
  await toggle(page);
  await toggle(page);
  await expect(input).toHaveText('@alpha Manual', { useInnerText: true });
  await expect(page.locator('.agent-chip[data-variant="chip"]')).toHaveCount(1);
  await input.fill('@');
  await page.getByRole('option', { name: /@alpha/ }).click();
  await toggle(page);
  await toggle(page);
  await expect(page.locator('.agent-chip[data-variant="chip"]')).toHaveCount(1);
  await expect(input).toHaveText('@alpha ', { useInnerText: true });
  expect(await run(page, 'proof')).toMatchObject({ writes: 0, preparations: 0 });
});
test('page/viewer preference persists across reload and surfaces, with isolated scopes', async ({
  page,
}) => {
  await mount(page, 'chat');
  await toggle(page);
  let input = await mount(page, 'reply');
  expect(await creatorOn(page)).toBe(false);
  await expect(input).toHaveText('', { useInnerText: true });
  input = await mount(page, 'annotation', { scope: 'different-page' });
  expect(await creatorOn(page)).toBe(true);
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
  await toggle(page);
  await expect(input).toHaveText('@alpha Saved', { useInnerText: true });
  await expect(page.locator('.agent-chip[data-variant="chip"]')).toHaveCount(1);
});
test('an offline saved creator stays seeded, and an offline agent on an online machine receives', async ({
  page,
}) => {
  const unavailable = await mount(page, 'chat', {
    agents: [{ ...alpha, online: 'offline' }],
  });
  expect(await hasCreatorRow(page)).toBe(true);
  await expect(unavailable).toHaveText('@alpha ', { useInnerText: true });
  await unavailable.pressSequentially('Waiting');
  await expect(
    page.getByRole('button', { name: /^Send(?: · to .+)?$/, exact: true }),
  ).toBeDisabled();
  expect(await run(page, 'proof')).toMatchObject({ writes: 0, preparations: 0 });

  const input = await mount(page, 'chat', {
    agents: [{ ...alpha, presence: 'offline' }],
  });
  expect(await hasCreatorRow(page)).toBe(true);
  await expect(input).toHaveText('@alpha ', { useInnerText: true });
  await input.pressSequentially('Inbox');
  await input.press('Enter');
  await expect
    .poll(() => run(page, 'proof'))
    .toMatchObject({ writes: 1, preparations: 1, commits: 1 });
  await expect(input).toHaveText('@alpha ', { useInnerText: true });
});
test('directory absence or ambiguity retains the metadata chip and cannot invent a send target', async ({
  page,
}) => {
  for (const agents of [
    [],
    [{ ...alpha, agent: id(9) }],
    [alpha, alpha],
    [alpha, { ...alpha, online: 'offline' as typeof alpha.online }],
  ]) {
    const input = await mount(page, 'chat', { agents });
    expect(await hasCreatorRow(page)).toBe(true);
    await expect(input).toHaveText('@alpha ', { useInnerText: true });
    await input.pressSequentially('Comment');
    await input.press('Enter');
    expect(await run(page, 'proof')).toMatchObject({ writes: 0, preparations: 0 });
  }
});
for (const variant of [
  { width: 1440, theme: 'light' },
  { width: 390, theme: 'dark' },
] as const) {
  test(`creator row in the agent list ${variant.width} ${variant.theme}`, async ({ page }) => {
    await page.setViewportSize({ width: variant.width, height: 900 });
    await page.emulateMedia({ colorScheme: variant.theme });
    await mount(page);
    await pill(page).click();
    await expect(creatorRow(page)).toHaveAttribute('aria-selected', 'true');
    await expect(creatorRow(page)).toBeVisible();
    // The list stays inside the viewport at any width.
    const geometry = await page.getByRole('listbox').evaluate((node) => {
      const r = node.getBoundingClientRect();
      return { left: r.left, right: r.right };
    });
    expect(geometry.left).toBeGreaterThanOrEqual(0);
    expect(geometry.right).toBeLessThanOrEqual(variant.width);
    await page.keyboard.press('Escape');
    await expect(page.getByRole('button', { name: 'Send', exact: true })).toBeVisible();
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
  await toggle(page);
  expect(await creatorOn(page)).toBe(false);
  await input.fill('Local only');
  await input.press('Enter');
  await expect.poll(() => run(page, 'proof')).toMatchObject({ writes: 1, preparations: 0 });
  await expect(input).toHaveText('', { useInnerText: true });
});
test('a preference changed during Send affects only the next draft, not the frozen message', async ({
  page,
}) => {
  // The sending composer's own toolbar is fenced; the page's other composer changes the preference.
  const input = await mount(page, 'linked');
  await expect(input).toHaveText('@alpha ', { useInnerText: true });
  await run(page, 'holdWrite');
  await input.evaluate((node) => {
    node.focus();
    document.getSelection()!.selectAllChildren(node);
    document.getSelection()!.collapseToEnd();
  });
  await input.pressSequentially('Frozen');
  await page.getByRole('button', { name: 'Send', exact: true }).first().click();
  await expect(input).toHaveAttribute('contenteditable', 'false');
  await expect(pill(page).first()).toBeDisabled();
  await toggle(page, 1);
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
    expect(await creatorOn(page)).toBe(true);
    await expect(input.locator('.agent-chip[data-variant="chip"]')).toHaveCount(1);
    await capture('on');
    await toggle(page);
    await expect(input.locator('.agent-chip[data-variant="chip"]')).toHaveCount(0);
    await capture('off');
    await toggle(page);
    await input.getByRole('button', { name: /^Remove mention:/ }).click();
    await input.pressSequentially('A draft stays here.');
    // A manual comment keeps the saved default in the list; it returns on a fresh draft.
    await expect(page.getByText(commentOnly, { exact: true })).toBeVisible();
    await capture('removed');
    await page.getByRole('button', { name: 'Send', exact: true }).click();
    expect(await creatorOn(page)).toBe(true);
    await expect(input.locator('.agent-chip[data-variant="chip"]')).toHaveCount(1);
    await capture('fresh');
    await open(false);
    expect(await hasCreatorRow(page)).toBe(false);
    await capture('hidden');
  });
}

test('a toggled seed alone is not a persisted draft; body edits are', async ({ page }) => {
  const input = await mount(page);
  await toggle(page);
  await toggle(page);
  await expect.poll(() => run(page, 'editingProof')).toMatchObject({ edit: { edited: false } });
  await input.pressSequentially('Keep this body');
  await expect.poll(() => run(page, 'editingProof')).toMatchObject({ edit: { edited: true } });
});
test('one page preference changes both mounted composers without publishing', async ({ page }) => {
  await mount(page, 'linked');
  const chips = page.locator('.agent-chip[data-variant="chip"]');
  await expect(chips).toHaveCount(2);
  await toggle(page, 0);
  expect(await creatorOn(page, 1)).toBe(false);
  await expect(chips).toHaveCount(0);
  await toggle(page, 1);
  expect(await creatorOn(page, 0)).toBe(true);
  await expect(chips).toHaveCount(2);
  expect(await run(page, 'proof')).toMatchObject({ writes: 0, preparations: 0 });
});
test('missing creation identity hides the control and leaves a plain comment', async ({ page }) => {
  const input = await mount(page, 'chat', { creator: false });
  expect(await hasCreatorRow(page)).toBe(false);
  await expect(input.locator('.agent-chip[data-variant="chip"]')).toHaveCount(0);
  await input.fill('Plain comment');
  await input.press('Enter');
  await expect.poll(() => run(page, 'proof')).toMatchObject({ writes: 1, preparations: 0 });
});

const commentOnly = 'Comment only, no agent is asked · Enter to send';
for (const surface of ['chat', 'annotation']) {
  for (const selected of [false, true]) {
    test(`${surface}: a creator choice made while the directory loads is fenced, and picking the agent preserves ${selected ? 'selection' : 'caret'} and only explicit Send asks`, async ({
      page,
    }) => {
      const input = await mount(page, surface, { directory: 'park' });
      await input.fill('Typed before the directory');
      await expect(pill(page)).toBeEnabled();
      await expect(page.locator('.annotation-hint').first()).toHaveText('Checking for agents…');
      expect(await run(page, 'proof')).toMatchObject({ writes: 0, preparations: 0, commits: 0 });
      await run(page, 'releaseDirectory');
      await expect(pill(page)).toBeEnabled();
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
        // The keyboard path: open the list on the pill, take its first agent.
        await pill(page).focus();
        await pill(page).press('Enter');
        await pill(page).press('Enter');
      } else await pickAlpha(page);
      await expect(input).toBeFocused();
      await expect(input).toHaveText('@alpha Typed before the directory', { useInnerText: true });
      await expect(page.locator('.agent-chip[data-variant="chip"]')).toHaveCount(1);
      await expect(page.locator('.annotation-hint').first()).toHaveText(
        'Asks alpha · Enter to send',
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
test('the metadata creator row remains available without replacing restored mention state', async ({
  page,
}) => {
  for (const agents of [[], [alpha, alpha], [{ ...alpha, agent: id(9) }]]) {
    const input = await mount(page, 'chat', {
      agents,
      initialEdit: { value: 'Restored plain draft', edited: true },
    });
    expect(await hasCreatorRow(page)).toBe(true);
    await expect(input).toHaveText('Restored plain draft');
  }
  await mount(page, 'chat', { initialEdit: { value: '@missing draft', edited: true } });
  await expect(page.locator('.annotation-hint').first()).toContainText('No agent named @missing');
});
test('a removed creator can be chosen again in the list without changing the saved default', async ({
  page,
}) => {
  const input = await mount(page);
  await page.getByRole('button', { name: /^Remove mention: @alpha/ }).click();
  await input.fill('A plain draft');
  await expect(page.locator('.annotation-hint').first()).toHaveText(commentOnly);
  await pickAlpha(page);
  await expect(input).toHaveText('@alpha A plain draft', { useInnerText: true });
  expect(await creatorOn(page)).toBe(true);
  await toggle(page);
  await expect(input).toHaveText('@alpha A plain draft', { useInnerText: true });
  expect(await run(page, 'proof')).toMatchObject({ writes: 0, preparations: 0, commits: 0 });
});

test('creator preference off keeps the one-off manual choice without changing the preference', async ({
  page,
}) => {
  const input = await mount(page);
  await toggle(page);
  expect(await creatorOn(page)).toBe(false);
  await input.fill('One-off request');
  await expect(page.locator('.annotation-hint').first()).toHaveText(commentOnly);
  await pickAlpha(page);
  await expect(input).toHaveText('@alpha One-off request', { useInnerText: true });
  await expect(input).toBeFocused();
  expect(await creatorOn(page)).toBe(false);
  expect(await run(page, 'proof')).toMatchObject({ writes: 0, preparations: 0, commits: 0 });
});

test('selecting the metadata seed before directory arrival keeps the selection and exact replacement', async ({
  page,
}) => {
  const input = await mount(page, 'chat', { directory: 'park' });
  await expect(input).toHaveText('@alpha ', { useInnerText: true });
  await input.evaluate((node) => {
    node.focus();
    document.getSelection()!.selectAllChildren(node);
  });
  await run(page, 'releaseDirectory');
  await expect(pill(page)).toBeEnabled();
  await page.keyboard.insertText('Replacement');
  await expect(input).toHaveText('Replacement', { useInnerText: true });
  await expect(input.locator('.agent-chip[data-variant="chip"]')).toHaveCount(0);
});

test('metadata creator is a chip on the first visible frame while directory discovery is parked', async ({
  page,
}) => {
  await page.goto('/');
  await run(page, 'setSurface', 'chat');
  await run(page, 'setDirectory', 'park');
  await run(page, 'setAgents', [alpha]);
  await run(page, 'setCreatorMetadata', { agentId: alpha.agent, label: 'alpha' });
  await run(page, 'mount', 'accepted');
  await expect
    .poll(() => run(page, 'firstPaintProof'))
    .toMatchObject({ chips: 1, value: '@alpha ' });
  await expect(page.getByRole('button', { name: /^Remove mention: @alpha/ })).toHaveCount(1);
  expect(await run(page, 'proof')).toMatchObject({ writes: 0, preparations: 0 });
});

test('creator metadata seeds without an Ask binding and directory recovery cannot reset an edited draft', async ({
  page,
}) => {
  const input = await mount(page, 'chat', { talk: false, directory: 'park' });
  await expect(input).toHaveText('@alpha ', { useInnerText: true });
  await input.pressSequentially('Kept');
  await run(page, 'setCreatorMetadata', { agentId: alpha.agent, label: 'renamed' });
  await expect(input).toHaveText('@alpha Kept', { useInnerText: true });
  await expect(page.locator('.agent-chip[data-variant="chip"]')).toHaveCount(1);
  expect(await run(page, 'proof')).toMatchObject({ writes: 0, preparations: 0 });
});

test('late creator metadata fills only an untouched draft', async ({ page }) => {
  for (const edited of [false, true]) {
    const input = await mount(page, 'chat', { creator: false });
    if (edited) await input.pressSequentially('Kept draft');
    await run(page, 'setCreatorMetadata', { agentId: alpha.agent, label: 'alpha' });
    await expect(input).toHaveText(edited ? 'Kept draft' : '@alpha ', { useInnerText: true });
  }
});

test('parked metadata chip survives directory arrival without replacement or duplication', async ({
  page,
}) => {
  const input = await mount(page, 'chat', { directory: 'park' });
  await expect(input).toHaveText('@alpha ', { useInnerText: true });
  await input.pressSequentially('Kept');
  await input.evaluate((node) => Object.assign(window, { creatorComposerRoot: node }));
  const caret = await input.evaluate(() => document.getSelection()?.anchorOffset);
  await run(page, 'releaseDirectory');
  await expect(page.locator('.annotation-hint[role="status"]').first()).not.toContainText(
    'Checking',
  );
  await expect(input).toHaveText('@alpha Kept', { useInnerText: true });
  await expect(page.locator('.agent-chip[data-variant="chip"]')).toHaveCount(1);
  expect(
    await input.evaluate(
      (node) =>
        (window as unknown as { creatorComposerRoot: Element }).creatorComposerRoot === node,
    ),
  ).toBe(true);
  expect(await input.evaluate(() => document.getSelection()?.anchorOffset)).toBe(caret);
});

test('creator absence is unknown, offline presence stays visible and uses the existing send path', async ({
  page,
}) => {
  for (const available of [false, true]) {
    const input = await mount(page, 'chat', {
      agents: available ? [{ ...alpha, presence: 'offline' }] : [],
    });
    await expect(input).toHaveText('@alpha ', { useInnerText: true });
    await expect(page.getByRole('button', { name: /^Remove mention: @alpha/ })).toHaveAttribute(
      'aria-label',
      available ? /Offline/ : /unknown/i,
    );
    await input.pressSequentially('Explain');
    await input.press('Enter');
    if (available)
      await expect
        .poll(() => run(page, 'proof'))
        .toMatchObject({ writes: 1, preparations: 1, commits: 1 });
    else expect(await run(page, 'proof')).toMatchObject({ writes: 0, preparations: 0 });
  }
});

test('opt-out and deliberately empty drafts win before directory arrival', async ({ page }) => {
  await mount(page);
  await toggle(page);
  let input = await mount(page, 'chat', { directory: 'park' });
  await expect(input).toHaveText('');
  await run(page, 'releaseDirectory');
  await expect(pill(page)).toBeEnabled();
  await expect(input.locator('.agent-chip[data-variant="chip"]')).toHaveCount(0);
  input = await mount(page, 'chat', {
    scope: 'edited-empty',
    directory: 'park',
    initialEdit: { value: '', edited: true },
  });
  await expect(input).toHaveText('');
  await run(page, 'releaseDirectory');
  await expect(pill(page)).toBeEnabled();
  await expect(input.locator('.agent-chip[data-variant="chip"]')).toHaveCount(0);
});

test('an identity-only creator binds once and keeps its original token across directory rename', async ({
  page,
}) => {
  await page.goto('/');
  await run(page, 'setSurface', 'chat');
  await run(page, 'setDirectory', 'park');
  await run(page, 'setAgents', [{ ...alpha, agentName: 'renamed' }]);
  await run(page, 'setCreatorMetadata', { agentId: alpha.agent, label: 'alpha' });
  await run(page, 'mount', 'accepted');
  const input = page.getByRole('combobox', { name: 'Message', exact: true });
  await expect(input.locator('.agent-chip[data-variant="chip"]')).toHaveCount(1);
  await input.pressSequentially('Explain');
  await run(page, 'releaseDirectory');
  await expect(page.locator('.annotation-hint[role="status"]')).not.toContainText('Checking');
  await expect(input).toHaveText('@alpha Explain', { useInnerText: true });
  await input.press('Enter');
  await expect
    .poll(() => run(page, 'proof'))
    .toMatchObject({ writes: 1, preparations: 1, commits: 1 });
  expect((await run(page, 'proof')).sends[0].agentId).toBe(alpha.agent);
});
