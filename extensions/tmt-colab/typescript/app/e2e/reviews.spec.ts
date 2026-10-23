import { expect, test, type Page } from '@playwright/test';
import { pageAction } from '../test/page-actions.js';
import { capturePath } from './captures.js';
import { expectAnswered } from './question-line.js';

const fixture = '/test/ask-page-browser.tsx';
async function run(page: Page, method: string, argument?: unknown) {
  return page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
}
const frame = (page: Page) => page.frameLocator('#ask-page-fixture iframe');
const section = (page: Page, index = 1) =>
  frame(page)
    .locator('tmt-review')
    .filter({ hasText: `Content of section ${index}.` });

// A card state commits before its new height has completed the renderer's slot round trip.
// Wait for the measured spacer AND the returned placement before targeting a sibling control.
async function reviewLayout(page: Page) {
  await expect
    .poll(async () => {
      const spacers = await frame(page)
        .locator('tmt-review')
        .evaluateAll((nodes) =>
          nodes.map((node) => {
            const spacer = node.nextElementSibling as HTMLElement | null;
            return {
              id: node.getAttribute('data-id'),
              height: spacer ? Number.parseFloat(spacer.style.minHeight) : null,
              top: spacer ? spacer.getBoundingClientRect().top + scrollY : null,
            };
          }),
        );
      return page.locator('[data-inline-review]').evaluateAll(
        (nodes, spacers) =>
          nodes.flatMap((node) => {
            const card = node as HTMLElement;
            const spacer = spacers.find((value) => value.id === card.dataset.inlineReview);
            const height = Math.ceil(card.getBoundingClientRect().height);
            const top = Number.parseFloat(card.style.top);
            return spacer?.height === height &&
              spacer.top !== null &&
              Math.abs(spacer.top - top) < 1
              ? []
              : [{ id: card.dataset.inlineReview, height, top, spacer }];
          }),
        spacers,
      );
    })
    .toEqual([]);
}

async function mount(page: Page, options = {}) {
  await page.goto('/');
  await run(page, 'mount');
  await run(page, 'review', options);
  await expect(page.locator('[data-inline-review]').first()).not.toHaveAttribute('data-detached');
  return page.locator('.review-card').first();
}
async function sectionThread(page: Page) {
  const threads = await run(page, 'discussionProof');
  return threads.filter((thread: { anchor: { exact: string } | null }) =>
    thread.anchor?.exact.startsWith('Section'),
  );
}

test('review controls wait for the changed sibling spacer and placement', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 1100 });
  await mount(page, { sections: 3 });
  await reviewLayout(page);
  await page.evaluate(() => {
    const post = MessagePort.prototype.postMessage;
    const pending: (() => void)[] = [];
    let released = false;
    MessagePort.prototype.postMessage = function (message, options) {
      const send = () => Reflect.apply(post, this, [message, options]);
      if (
        !released &&
        message?.type === 'colab.render.slots' &&
        document.querySelector('[data-review-state="changes"]')
      )
        pending.push(send);
      else send();
    };
    Reflect.set(window, 'reviewSlotGate', {
      held: () => pending.length,
      release: () => {
        released = true;
        pending.splice(0).forEach((send) => send());
        MessagePort.prototype.postMessage = post;
      },
    });
  });
  const bands = page.locator('.review-card');
  await bands.nth(1).getByRole('button', { name: 'Needs change…', exact: true }).click();
  await bands.nth(1).getByLabel('What needs to change', { exact: true }).fill('A shorter section.');
  await bands.nth(1).getByRole('button', { name: 'Send', exact: true }).click();
  await expect(bands.nth(1)).toHaveAttribute('data-review-state', 'changes');
  await expect
    .poll(() => page.evaluate(() => Reflect.get(window, 'reviewSlotGate').held()))
    .toBeGreaterThan(0);
  let settled = false;
  const ready = reviewLayout(page).then(() => {
    settled = true;
  });
  // A browser round trip lets the barrier inspect the held layout; state alone is not readiness.
  await page.evaluate(() => Reflect.get(window, 'reviewSlotGate').held());
  expect(settled).toBe(false);
  await page.evaluate(() => Reflect.get(window, 'reviewSlotGate').release());
  await ready;
  await bands.nth(2).getByRole('button', { name: 'Add a note', exact: true }).click();
  await bands.nth(2).getByLabel('Note (optional)', { exact: true }).fill('Looks right to me');
  await expect(bands.nth(2).getByLabel('Note (optional)', { exact: true })).toHaveValue(
    'Looks right to me',
  );
});

test('a waiting section offers Confirm and Needs change, and the head counts it', async ({
  page,
}) => {
  const card = await mount(page, { sections: 2 });
  await expect(card).toHaveAttribute('data-review-state', 'waiting');
  await expect(card.getByText('Please confirm')).toBeVisible();
  await expect(card.getByRole('button', { name: 'Confirm and close', exact: true })).toBeEnabled();
  await expect(card.getByRole('button', { name: 'Needs change…', exact: true })).toBeEnabled();
  await expect(card.getByText('Goes to Agent 1')).toBeVisible();
  const chip = page.getByRole('button', { name: 'Needs you: 2', exact: true });
  await expect(chip.locator('.needs-you-wide')).toHaveText('Needs you');
  await expect(chip.locator('.tmt-ui-count')).toHaveText('2');
  await expect(section(page)).toHaveAttribute('data-colab-review-closed', 'false');
  expect(await sectionThread(page)).toHaveLength(0);
});

test('Confirm and close records one comment, resolves the thread and fades the section in place', async ({
  page,
}) => {
  const card = await mount(page, { sections: 2 });
  // Synthetic activation has no authority.
  await card
    .getByRole('button', { name: 'Confirm and close', exact: true })
    .evaluate((node) => (node as HTMLButtonElement).click());
  expect(await sectionThread(page)).toHaveLength(0);
  expect((await run(page, 'proof')).actions).toEqual([]);

  await card.getByRole('button', { name: 'Confirm and close', exact: true }).click();
  await expect(card).toHaveAttribute('data-review-state', 'closed');
  const threads = await sectionThread(page);
  expect(threads).toHaveLength(1);
  expect(threads[0].anchor.exact).toBe('Section 1');
  expect(threads[0].comments).toHaveLength(1);
  expect(threads[0].comments[0].body).toBe(
    'Confirmed “Section 1”\nPage: Live shared page\n@Agent 1',
  );
  expect((await run(page, 'proof')).actions).toEqual(['status:resolve']);
  await expect(page.getByRole('button', { name: 'Needs you: 1', exact: true })).toBeVisible();
  // Faded, never collapsed or hidden: its content stays visible and selectable.
  const closed = section(page);
  await expect(closed).toHaveAttribute('data-colab-review-closed', 'true');
  await expect(closed.getByText('Content of section 1.')).toBeVisible();
  expect(await closed.evaluate((node) => Number(getComputedStyle(node).opacity))).toBeLessThan(1);
  expect(await closed.evaluate((node) => getComputedStyle(node).display)).toBe('block');
  await expect(section(page, 2)).toHaveAttribute('data-colab-review-closed', 'false');
  // The closed band offers only Reopen.
  await expect(card.getByRole('button', { name: 'Confirm and close' })).toHaveCount(0);
  await card.getByRole('button', { name: 'Reopen', exact: true }).click();
  await expect(card).toHaveAttribute('data-review-state', 'waiting');
  await expect(section(page)).toHaveAttribute('data-colab-review-closed', 'false');
  expect((await run(page, 'proof')).actions).toEqual(['status:resolve', 'status:reopen']);
  await expect(page.getByRole('button', { name: 'Needs you: 2', exact: true })).toBeVisible();
});

test('Confirm keeps an optional one-line note in the closing comment', async ({ page }) => {
  const card = await mount(page);
  await card.getByRole('button', { name: 'Add a note', exact: true }).click();
  await card.getByLabel('Note (optional)', { exact: true }).fill('Looks right to me');
  await card.getByRole('button', { name: 'Confirm and close', exact: true }).click();
  await expect(card).toHaveAttribute('data-review-state', 'closed');
  await expect(card.getByText('Looks right to me')).toBeVisible();
  expect((await sectionThread(page))[0].comments[0].body).toBe(
    'Confirmed “Section 1”: Looks right to me\nPage: Live shared page\n@Agent 1',
  );
});

test('Needs change adds a comment and an Ask, keeps the thread open, and a reply returns it to waiting', async ({
  page,
}) => {
  const card = await mount(page);
  await card.getByRole('button', { name: 'Needs change…', exact: true }).click();
  const send = card.getByRole('button', { name: 'Send', exact: true });
  await expect(send).toBeDisabled();
  await card.getByLabel('What needs to change', { exact: true }).fill('Say what the page is for.');
  await send.click();
  await expect(card).toHaveAttribute('data-review-state', 'changes');
  await expect(card.getByText('Changes requested')).toBeVisible();
  await expect(card.locator('.review-note')).toHaveText('Say what the page is for.');
  // The composer empties once the Ask settled.
  await expect(card.getByLabel('What needs to change', { exact: true })).toHaveCount(0);
  const threads = await sectionThread(page);
  expect(threads).toHaveLength(1);
  expect(threads[0].comments[0].body).toBe(
    'Needs change to “Section 1”: Say what the page is for.\nPage: Live shared page\n@Agent 1',
  );
  await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(1);
  // No status action: the thread stays open, but the header excludes work waiting for the agent.
  expect((await run(page, 'proof')).actions).toEqual([]);
  await expect(
    page.locator('#ask-page-fixture').getByLabel('✓ Nothing waits', { exact: true }),
  ).toBeVisible();
  await expect(section(page)).toHaveAttribute('data-colab-review-closed', 'false');
  await run(page, 'questionAsk', 'replied');
  await expect(card).toHaveAttribute('data-review-state', 'waiting');
  await expect(card.getByRole('button', { name: 'Confirm and close', exact: true })).toBeEnabled();
});

test('an agent resolving the thread does not close the section; Confirm reopens, then resolves', async ({
  page,
}) => {
  const card = await mount(page);
  await card.getByRole('button', { name: 'Needs change…', exact: true }).click();
  await card.getByLabel('What needs to change', { exact: true }).fill('Fix the heading.');
  await card.getByRole('button', { name: 'Send', exact: true }).click();
  await expect(card).toHaveAttribute('data-review-state', 'changes');
  await run(page, 'agentResolves');
  // The agent-labelled Resolve leaves Changes requested; nothing yet waits for the reader.
  await expect(card).not.toHaveAttribute('data-review-state', 'closed');
  await expect(
    page.locator('#ask-page-fixture').getByLabel('✓ Nothing waits', { exact: true }),
  ).toBeVisible();
  await expect(section(page)).toHaveAttribute('data-colab-review-closed', 'false');
  await card.getByRole('button', { name: 'Confirm and close', exact: true }).click();
  await expect(card).toHaveAttribute('data-review-state', 'closed');
  expect((await run(page, 'proof')).actions).toEqual(['status:reopen', 'status:resolve']);
  await expect(
    page.locator('#ask-page-fixture').getByLabel('✓ Nothing waits', { exact: true }),
  ).toBeVisible();
});

test('page HTML cannot close a section, count or draw controls', async ({ page }) => {
  await page.goto('/');
  await run(page, 'mount');
  await run(page, 'review', { forged: true });
  const card = page.locator('.review-card').first();
  await expect(card).toHaveAttribute('data-review-state', 'waiting');
  // The page authored `data-colab-review-closed="true"`; the frame follows the parent's state.
  await expect(section(page)).toHaveAttribute('data-colab-review-closed', 'false');
  // Authored text that looks like a status changes nothing in the parent.
  await expect(frame(page).getByText('✓ Closed by the author')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Needs you: 1', exact: true })).toBeVisible();
  // Page-authored buttons in the frame cannot reach the parent's action.
  await frame(page).getByRole('button', { name: 'Confirm and close' }).click();
  expect(await sectionThread(page)).toHaveLength(0);
  expect((await run(page, 'proof')).actions).toEqual([]);
});

test('duplicate and nested sections supply no controls', async ({ page }) => {
  await page.goto('/');
  await run(page, 'mount');
  await run(page, 'review', { placement: 'duplicate' });
  await expect(page.locator('.review-card')).toHaveCount(0);
  await expect(page.getByRole('button', { name: /^Needs you:/ })).toHaveCount(0);
  await run(page, 'review', { placement: 'nested' });
  await expect(page.locator('.review-card')).toHaveCount(1);
  await expect(page.getByRole('button', { name: 'Needs you: 1', exact: true })).toBeVisible();
});

test('answering the questions inside a section closes it in the same Send', async ({ page }) => {
  const card = await mount(page, { question: true });
  await expect(card).toHaveAttribute('data-review-state', 'waiting');
  const question = page.locator('.question-group-card');
  await question.locator('.question-line').click();
  await question.getByLabel('Your answer', { exact: true }).fill('Use a clearer heading.');
  await question.getByRole('button', { name: /^Send answer/ }).press('Enter');
  await expect(card).toHaveAttribute('data-review-state', 'closed');
  const threads = await run(page, 'discussionProof');
  const answers = threads.filter((thread: { anchor: { exact: string } }) =>
    thread.anchor.exact.startsWith('Which heading'),
  );
  const sections = threads.filter((thread: { anchor: { exact: string } }) =>
    thread.anchor.exact.startsWith('Section'),
  );
  expect(answers).toHaveLength(1);
  expect(sections).toHaveLength(1);
  expect(sections[0].comments[0].body).toBe('Answered “Section 1”\nPage: Live shared page');
  expect((await run(page, 'proof')).actions).toEqual(['status:resolve']);
  await expect(
    page.locator('#ask-page-fixture').getByLabel('✓ Nothing waits', { exact: true }),
  ).toBeVisible();
});

test('an archived page shows section state and count but no controls', async ({ page }) => {
  const card = await mount(page);
  await card.getByRole('button', { name: 'Confirm and close', exact: true }).click();
  await expect(card).toHaveAttribute('data-review-state', 'closed');
  await run(page, 'archive');
  await expect(card.getByRole('button', { name: 'Reopen' })).toHaveCount(0);
  await expect(card.locator('.needs-you-bar strong')).toHaveText('Closed');
  await expect(
    page.locator('#ask-page-fixture').getByLabel('✓ Nothing waits', { exact: true }),
  ).toBeVisible();
});

test('a Resolve that did not land leaves the section waiting; the retry publishes only the Resolve', async ({
  page,
}) => {
  const card = await mount(page);
  await run(page, 'failStatusOnce');
  await card.getByRole('button', { name: 'Confirm and close', exact: true }).click();
  await expect(card.getByRole('alert')).toContainText('Your comment is saved');
  await expect(card).toHaveAttribute('data-review-state', 'waiting');
  expect((await run(page, 'proof')).actions).toEqual([]);
  await expect(page.getByRole('button', { name: 'Needs you: 1', exact: true })).toBeVisible();
  await card.getByRole('button', { name: 'Confirm and close', exact: true }).click();
  await expect(card).toHaveAttribute('data-review-state', 'closed');
  // The confirmation is recorded once, not twice.
  const threads = await sectionThread(page);
  expect(threads).toHaveLength(1);
  expect(threads[0].comments).toHaveLength(1);
  expect((await run(page, 'proof')).actions).toEqual(['status:resolve']);
});

test('concurrent first actions make sibling threads and every tab agrees on one canonical thread', async ({
  page,
}) => {
  const card = await mount(page);
  await card.getByRole('button', { name: 'Needs change…', exact: true }).click();
  await card.getByLabel('What needs to change', { exact: true }).fill('Fix the heading.');
  await card.getByRole('button', { name: 'Send', exact: true }).click();
  await expect(card).toHaveAttribute('data-review-state', 'changes');
  // Another reader confirmed at the same time: its thread orders first, so it is canonical.
  await run(page, 'reviewSibling', {
    body: 'Confirmed “Section 1”\nPage: Live shared page',
    resolved: 'person',
    writer: 2,
  });
  await expect(card).toHaveAttribute('data-review-state', 'closed');
  await expect(
    page.locator('#ask-page-fixture').getByLabel('✓ Nothing waits', { exact: true }),
  ).toBeVisible();
  // An agent-labelled sibling never closes anything.
  await run(page, 'reviewSibling', {
    body: 'Confirmed “Section 1”\nPage: Live shared page',
    resolved: 'agent',
    writer: 1,
  });
  await expect(card).not.toHaveAttribute('data-review-state', 'closed');
  await expect(page.getByRole('button', { name: 'Needs you: 1', exact: true })).toBeVisible();
});

test('a group inside a section closes it only when every question is answered', async ({
  page,
}) => {
  const card = await mount(page, { group: true });
  await expect(page.getByRole('button', { name: 'Needs you: 2', exact: true })).toBeVisible();
  const group = page.locator('.question-group-card');
  const active = page.locator('.question-card:visible').first();
  await group.locator('.question-line').first().click();
  await active.getByLabel('Your answer', { exact: true }).fill('First answer.');
  await group.getByRole('button', { name: /^Send/ }).press('Enter');
  await expectAnswered(group.locator('.question-line').first(), 'First answer.');
  await expect(page.getByRole('button', { name: 'Needs you: 2', exact: true })).toBeVisible();
  // One of two answered: the section stays open and nothing was resolved.
  await expect(card).toHaveAttribute('data-review-state', 'waiting');
  expect((await run(page, 'proof')).actions).toEqual([]);
  await group.locator('.question-line').nth(1).click();
  await active.getByLabel('Your answer', { exact: true }).fill('Second answer.');
  await group.getByRole('button', { name: /^Send/ }).press('Enter');
  await expect(card).toHaveAttribute('data-review-state', 'closed');
  expect((await run(page, 'proof')).actions).toEqual(['status:resolve']);
  await expect(
    page.locator('#ask-page-fixture').getByLabel('✓ Nothing waits', { exact: true }),
  ).toBeVisible();
});

test('the controls wait for the asking agent to resolve, then name it', async ({ page }) => {
  await page.goto('/');
  await run(page, 'mount', { holdQuestionDirectory: true });
  await run(page, 'review');
  const card = page.locator('.review-card').first();
  await expect(card.getByRole('button', { name: 'Confirm and close', exact: true })).toBeDisabled();
  await run(page, 'finishQuestionDirectory');
  await expect(card.getByRole('button', { name: 'Confirm and close', exact: true })).toBeEnabled();
  await expect(card.getByText('Goes to Agent 1')).toBeVisible();
});

test('an unknown asking agent keeps the comment on the page and says so', async ({ page }) => {
  const card = await mount(page, { agent: 'Missing agent' });
  await expect(card.getByText('Stays on the page, no one is asked')).toBeVisible();
  await card.getByRole('button', { name: 'Confirm and close', exact: true }).click();
  await expect(card).toHaveAttribute('data-review-state', 'closed');
  expect((await sectionThread(page))[0].comments[0].body).toBe(
    'Confirmed “Section 1”\nPage: Live shared page',
  );
});

test('only a thread on the whole heading is the section thread; a partial note is an ordinary annotation', async ({
  page,
}) => {
  const card = await mount(page);
  await card.getByRole('button', { name: 'Needs change…', exact: true }).click();
  await card.getByLabel('What needs to change', { exact: true }).fill('Fix the heading.');
  await card.getByRole('button', { name: 'Send', exact: true }).click();
  await expect(card).toHaveAttribute('data-review-state', 'changes');
  // A resolved confirmation-looking thread on part of the heading, and one inside the content.
  await run(page, 'reviewSibling', {
    body: 'Confirmed “Section 1”\nPage: Live shared page',
    resolved: 'person',
    writer: 2,
    anchor: { exact: 'Section', prefix: '', suffix: '' },
  });
  await run(page, 'reviewSibling', {
    body: 'Confirmed “Section 1”\nPage: Live shared page',
    resolved: 'person',
    writer: 1,
    anchor: { exact: 'Content of section 1.', prefix: '', suffix: '' },
  });
  await expect(card).toHaveAttribute('data-review-state', 'changes');
  await expect(
    page.locator('#ask-page-fixture').getByLabel('✓ Nothing waits', { exact: true }),
  ).toBeVisible();
});

/** WCAG contrast of `foreground` blended over `background` at `opacity`, against the background. */
function fadedContrast(foreground: number[], background: number[], opacity: number) {
  const blended = foreground.map(
    (value, index) => opacity * value + (1 - opacity) * background[index],
  );
  const luminance = (color: number[]) => {
    const [r, g, b] = color.map((value) => {
      const unit = value / 255;
      return unit <= 0.03928 ? unit / 12.92 : ((unit + 0.055) / 1.055) ** 2.4;
    });
    return 0.2126 * r + 0.7152 * g + 0.0722 * b;
  };
  const [light, dark] = [luminance(blended), luminance(background)].sort((a, b) => b - a);
  return (light + 0.05) / (dark + 0.05);
}
const rgb = (value: string) => (value.match(/[\d.]+/g) ?? []).slice(0, 3).map(Number);

for (const [width, theme] of [
  [1440, 'light'],
  [1440, 'dark'],
  [390, 'light'],
  [390, 'dark'],
] as const) {
  for (const darkPage of [false, true]) {
    test(`section look ${width} ${theme} page-${darkPage ? 'dark' : 'light'}: three states, no overflow, readable fade`, async ({
      page,
    }, testInfo) => {
      await page.setViewportSize({ width, height: 1100 });
      await page.goto('/');
      await run(page, 'mount', { statusAgents: true });
      await run(page, 'review', { sections: 3, darkPage });
      await (
        await pageAction(
          page.locator('#ask-page-fixture'),
          theme === 'dark' ? 'Theme: Dark' : 'Theme: Light',
        )
      ).click();
      await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
      const bands = page.locator('.review-card');
      await expect(page.locator('[data-inline-review]').nth(2)).not.toHaveAttribute(
        'data-detached',
      );
      // Section 2 asks for a change, section 3 is confirmed; section 1 keeps waiting.
      await bands.nth(1).getByRole('button', { name: 'Needs change…', exact: true }).click();
      await bands
        .nth(1)
        .getByLabel('What needs to change', { exact: true })
        .fill('Say what this section is for, in one sentence.');
      await bands.nth(1).getByRole('button', { name: 'Send', exact: true }).click();
      await expect(bands.nth(1)).toHaveAttribute('data-review-state', 'changes');
      await reviewLayout(page);
      await bands.nth(2).getByRole('button', { name: 'Add a note', exact: true }).click();
      await bands.nth(2).getByLabel('Note (optional)', { exact: true }).fill('Looks right to me');
      await bands.nth(2).getByRole('button', { name: 'Confirm and close', exact: true }).click();
      await expect(bands.nth(2)).toHaveAttribute('data-review-state', 'closed');
      await expect(page.getByRole('button', { name: 'Needs you: 1', exact: true })).toBeVisible();
      // Nothing overflows at this width, and the bands sit below their sections.
      for (let index = 0; index < 3; index++) {
        await expect(page.locator('[data-inline-review]').nth(index)).not.toHaveAttribute(
          'data-detached',
        );
        const box = await bands.nth(index).evaluate((node) => {
          const rect = node.getBoundingClientRect();
          return {
            inside: rect.left >= -1 && rect.right <= innerWidth + 1,
            clipped: node.scrollWidth > node.clientWidth + 1,
          };
        });
        expect(box, `band ${index + 1}`).toEqual({ inside: true, clipped: false });
      }
      // The shared needs-you look: no frame, a light fill band only while the section needs the
      // reader, only the waiting ◆ in the attention colour, and neutral lines without a fill for a
      // section that waits for the agent or is settled.
      const look = await page.evaluate(() => {
        const probe = document.createElement('i');
        document.body.append(probe);
        const resolve = (property: 'backgroundColor' | 'color', token: string) => {
          probe.style[property] = `var(${token})`;
          return getComputedStyle(probe)[property];
        };
        const band = resolve('backgroundColor', '--tmt-ui-surface-mention-agent');
        const waiting = resolve('color', '--tmt-ui-color-waiting');
        const secondary = resolve('color', '--tmt-ui-color-secondary');
        probe.remove();
        return [...document.querySelectorAll('.review-card')].map((card) => {
          const bar = card.querySelector('.needs-you-bar')!;
          const style = getComputedStyle(bar);
          const frame = getComputedStyle(card);
          const actions = [...bar.querySelectorAll('.needs-you-actions button')].map((button) =>
            button.getBoundingClientRect(),
          );
          const first = actions[0];
          const edge = bar.getBoundingClientRect().left + parseFloat(style.paddingLeft);
          const mark = bar.querySelector('.needs-you-mark')!;
          return {
            band,
            waiting,
            secondary,
            frame: [frame.borderTopWidth, frame.borderLeftWidth],
            fill: style.backgroundColor,
            color: style.color,
            mark: mark.textContent,
            markColor: getComputedStyle(mark).color,
            // Offsets of every action that sits on a later line than the first one.
            wrapped: actions
              .filter((rect) => first && rect.top > first.top + 1)
              .map((rect) => Math.abs(rect.left - edge)),
            // Where the actions start, as an offset from the text edge, when they have a line of their own.
            own: first && first.top > bar.getBoundingClientRect().top + 12 ? first.left - edge : 0,
          };
        });
      });
      for (const [index, band] of look.entries()) {
        expect(band.frame, `band ${index + 1} has no frame`).toEqual(['0px', '0px']);
        for (const offset of band.wrapped)
          expect(offset, `band ${index + 1} wrapped action`).toBeLessThanOrEqual(1);
        expect(band.own, `band ${index + 1} own-line action`).toBeLessThanOrEqual(1);
      }
      // Section 1 waits for the reader, 2 waits for the agent, 3 is settled.
      expect(look.map((band) => band.mark)).toEqual(['◆', '◇', '✓']);
      expect(look[0].fill).toBe(look[0].band);
      expect(look[0].markColor).toBe(look[0].waiting);
      expect(look[1].fill).toBe('rgba(0, 0, 0, 0)');
      expect(look[1].markColor).toBe(look[1].secondary);
      expect(look[1].color).toBe(look[1].secondary);
      expect(look[2].fill).toBe('rgba(0, 0, 0, 0)');
      expect(look[2].color).toBe(look[2].secondary);
      const closed = frame(page).locator('tmt-review').nth(2);
      const style = await closed.evaluate((node) => {
        const text = node.querySelector('p')!;
        return {
          opacity: Number(getComputedStyle(node).opacity),
          color: getComputedStyle(text).color,
          background: getComputedStyle(document.body).backgroundColor,
        };
      });
      const contrast = fadedContrast(rgb(style.color), rgb(style.background), style.opacity);
      testInfo.annotations.push({ type: 'faded-contrast', description: contrast.toFixed(2) });
      // Faded, not hidden: the closed content stays at least large-text readable.
      expect(contrast).toBeGreaterThanOrEqual(3);
      await expect(closed.getByText('Content of section 3.')).toBeVisible();
      await page.screenshot({
        fullPage: true,
        path: capturePath(
          `review-sections-${width}-${theme}-page-${darkPage ? 'dark' : 'light'}.png`,
        ),
      });
    });
  }
}

test('Confirm tells the asking agent once; Reopen and a change request send no status notice', async ({
  page,
}) => {
  await page.goto('/');
  await run(page, 'mount', { statusAgents: true });
  await run(page, 'review');
  const card = page.locator('.review-card').first();
  await expect(card.getByRole('button', { name: 'Confirm and close', exact: true })).toBeEnabled();
  await card.getByRole('button', { name: 'Confirm and close', exact: true }).click();
  await expect(card).toHaveAttribute('data-review-state', 'closed');
  await expect
    .poll(async () => (await run(page, 'proof')).actions)
    .toEqual(['status:resolve', 'notify:Agent 1']);
  // The comment carried the mention but sent no Ask of its own.
  expect((await run(page, 'proof')).sends).toHaveLength(0);
  await card.getByRole('button', { name: 'Reopen', exact: true }).click();
  await expect(card).toHaveAttribute('data-review-state', 'waiting');
  expect((await run(page, 'proof')).actions).toEqual([
    'status:resolve',
    'notify:Agent 1',
    'status:reopen',
  ]);
  await card.getByRole('button', { name: 'Needs change…', exact: true }).click();
  await card.getByLabel('What needs to change', { exact: true }).fill('One more thing.');
  await card.getByRole('button', { name: 'Send', exact: true }).click();
  await expect(card).toHaveAttribute('data-review-state', 'changes');
  await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(1);
  expect((await run(page, 'proof')).actions).toHaveLength(3);
});

test('a Reopen that did not land says so, keeps the section closed and can be retried', async ({
  page,
}) => {
  const card = await mount(page);
  await card.getByRole('button', { name: 'Confirm and close', exact: true }).click();
  await expect(card).toHaveAttribute('data-review-state', 'closed');
  await run(page, 'failStatusOnce');
  await card.getByRole('button', { name: 'Reopen', exact: true }).click();
  await expect(card.getByRole('alert')).toHaveText('Could not reopen this section. Try again.');
  await expect(card).toHaveAttribute('data-review-state', 'closed');
  // The stale "comment saved" wording never appears on a closed section.
  await expect(card.getByText('Your comment is saved')).toHaveCount(0);
  await card.getByRole('button', { name: 'Reopen', exact: true }).click();
  await expect(card).toHaveAttribute('data-review-state', 'waiting');
  await expect(card.getByRole('alert')).toHaveCount(0);
});

test('a Reopen result that arrives after the page changed never reaches the next page', async ({
  page,
}) => {
  const card = await mount(page);
  await card.getByRole('button', { name: 'Confirm and close', exact: true }).click();
  await expect(card).toHaveAttribute('data-review-state', 'closed');
  await run(page, 'holdStatusOnce');
  await card.getByRole('button', { name: 'Reopen', exact: true }).click();
  await page.evaluate(() => (location.hash = '/pages/review-b'));
  await expect(frame(page).locator('#review-b-page')).toBeVisible();
  const next = page.locator('.review-card').first();
  await expect(next).toHaveAttribute('data-review-state', 'closed');
  await run(page, 'releaseStatus', { fail: true });
  // The durable coordinator stays the authority; the old page's failure is not shown here.
  await expect(next.getByRole('alert')).toHaveCount(0);
  await expect(next).toHaveAttribute('data-review-state', 'closed');
  await next.getByRole('button', { name: 'Reopen', exact: true }).click();
  await expect(next).toHaveAttribute('data-review-state', 'waiting');
});

test('a page with no Ask facade lets a reader confirm with the comment staying on the page', async ({
  page,
}) => {
  await page.goto('/');
  await run(page, 'mount', { commentOnly: true });
  await run(page, 'review');
  const card = page.locator('.review-card').first();
  await expect(card.getByText('Stays on the page, no one is asked')).toBeVisible();
  await card.getByRole('button', { name: 'Confirm and close', exact: true }).click();
  await expect(card).toHaveAttribute('data-review-state', 'closed');
  expect((await sectionThread(page))[0].comments[0].body).toBe(
    'Confirmed “Section 1”\nPage: Live shared page',
  );
});
