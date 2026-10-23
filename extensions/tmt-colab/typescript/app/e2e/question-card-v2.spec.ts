import { expect, test, type Locator, type Page } from '@playwright/test';
import { capturePath } from './captures.js';
import { expectOnePrimaryPerGroup } from './primary-groups.js';
import { rgb, tokens } from './tokens.js';

/**
 * The question card is the concept website's question block (#/colab/page/ops-ui): a band between
 * two rules, a header of ? glyph, eyebrow, title, state and chevron, a kind line and an instruction
 * over one wrapped row of choices, the Other field under them once Other is picked, and a rule over
 * a "Goes to" row with the one primary action.
 */
const fixture = '/test/ask-page-browser.tsx';
async function run(page: Page, method: string, argument?: unknown) {
  return page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
}
async function mount(page: Page, width: number, scheme: 'light' | 'dark') {
  await page.setViewportSize({ width, height: 900 });
  await page.emulateMedia({ colorScheme: scheme, reducedMotion: 'reduce' });
  await page.goto('/');
  await run(page, 'mount', {});
}
const luminance = (hex: string) => {
  const [r, g, b] = [1, 3, 5].map((i) => {
    const channel = parseInt(hex.slice(i, i + 2), 16) / 255;
    return channel <= 0.03928 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
};
const contrast = (a: string, b: string) => {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
};

/** A rounded element with one to three borders curls at the corner into a hook. */
async function oneSidedBorders(block: Locator) {
  return block.evaluate((node) =>
    [node, ...node.querySelectorAll<HTMLElement>('*')].flatMap((element) => {
      const style = getComputedStyle(element);
      const sides = ['top', 'right', 'bottom', 'left'].filter(
        (side) => parseFloat(style.getPropertyValue(`border-${side}-width`)) > 0,
      );
      const rounded = parseFloat(style.borderTopLeftRadius) > 0;
      return rounded && sides.length > 0 && sides.length < 4
        ? [`${element.className} ${sides.join('+')}`]
        : [];
    }),
  );
}

// The shard model has no room for more cases: two views cover both widths and both schemes.
for (const [width, scheme] of [
  [1440, 'light'],
  [390, 'dark'],
] as const) {
  test(`the question card is the website's band: header, kind line, choices, Other, Goes to, answered ${width} ${scheme}`, async ({
    page,
  }) => {
    await mount(page, width, scheme);
    await run(page, 'question', {
      kind: 'single',
      choices: ['Keep it', 'Change it'],
      other: true,
      darkPage: scheme === 'dark',
    });
    const card = page.locator('.question-group-card');
    const line = card.locator('.question-line');
    const block = card.locator('.question-compact-block');
    const sheet = tokens.browser.surface.sheet[scheme];

    // Closed: the header names the asking agent above the question, with the waiting state.
    await expect(line.locator('.question-line-icon')).toHaveText('?');
    await expect(line.locator('.question-line-eyebrow')).toHaveText('Q&A · Agent 1');
    await expect(line.locator('.question-line-title')).toHaveText('Which heading should we use?');
    await expect(line.locator('.question-line-state')).toHaveText('◆Needs your answer');
    await expect(line.locator('.lucide-chevron-down')).toHaveCount(1);
    await line.click();
    const controls = card.locator('.question-compact-controls:not([hidden])');
    await expect(controls).toBeVisible();
    await expect(line.locator('.lucide-chevron-up')).toHaveCount(1);

    // A band: the sheet between two rules, with no side rule, no radius and no shadow.
    await expect(block).toHaveCSS('background-color', rgb(sheet));
    for (const side of ['top', 'bottom']) {
      await expect(block).toHaveCSS(`border-${side}-width`, '1px');
      await expect(block).toHaveCSS(
        `border-${side}-color`,
        rgb(tokens.browser.surface.rule[scheme]),
      );
    }
    for (const side of ['left', 'right'])
      await expect(block).toHaveCSS(`border-${side}-width`, '0px');
    for (const corner of ['top-left', 'top-right', 'bottom-left', 'bottom-right'])
      await expect(block).toHaveCSS(`border-${corner}-radius`, '0px');
    await expect(block).toHaveCSS('box-shadow', 'none');
    expect(await oneSidedBorders(block)).toEqual([]);

    // The state sits at the right of the title (under it at 390), in the waiting colour.
    const [titleBox, stateBox, blockBox] = [
      await line.locator('.question-line-title').boundingBox(),
      await line.locator('.question-line-state').boundingBox(),
      await block.boundingBox(),
    ];
    if (width > 600) {
      expect(stateBox!.x).toBeGreaterThan(titleBox!.x + titleBox!.width - 1);
      expect(stateBox!.x + stateBox!.width).toBeLessThanOrEqual(blockBox!.x + blockBox!.width);
    } else expect(stateBox!.y).toBeGreaterThan(titleBox!.y + titleBox!.height - 1);
    await expect(line.locator('.question-line-state')).toHaveCSS(
      'color',
      rgb(tokens.browser.color.waiting[scheme]),
    );

    // The kind line and the instruction head the choices, which stand in one wrapped row.
    await expect(card.locator('.question-kind')).toHaveText('Single choice · Other available');
    const legend = card.locator('.tmt-ui-choices-legend');
    await expect(legend).toHaveText('Choose one answer, or write your own.');
    await expect(legend).toBeVisible();
    const radios = ['Keep it', 'Change it', 'Other'].map((name) =>
      card.getByRole('radio', { name, exact: true }),
    );
    const radioBoxes = await Promise.all(radios.map((radio) => radio.boundingBox()));
    const legendBox = await legend.boundingBox();
    expect(radioBoxes[0]!.y).toBeGreaterThan(legendBox!.y + legendBox!.height - 1);
    if (width > 600)
      for (const box of radioBoxes) expect(Math.abs(box!.y - radioBoxes[0]!.y)).toBeLessThan(2);
    if (width > 600) expect(Math.abs(radioBoxes[0]!.x - titleBox!.x)).toBeLessThanOrEqual(2);

    // Other: its field is not drawn until Other is picked, then it sits under the choices.
    const field = card.getByLabel('Your other answer', { exact: true });
    await expect(field).toHaveCount(0);
    await radios[2].check();
    await expect(field).toBeVisible();
    await expect(field).toHaveAttribute('placeholder', 'Tell us what is missing…');
    expect((await field.boundingBox())!.y).toBeGreaterThan(
      radioBoxes[2]!.y + radioBoxes[2]!.height,
    );
    await radios[0].check();
    await expect(field).toHaveCount(0);

    // The rule between the fields and the send row; "Goes to" and the one primary action.
    const footer = card.locator('.question-group-footer');
    await expect(footer).toHaveCSS('border-top-width', '1px');
    await expect(card.locator('.question-goes-to')).toHaveText('Goes to');
    const send = card.getByRole('button', { name: /^Send answer/ });
    await expect(send).toBeEnabled();
    await expect(send).toHaveAttribute('data-variant', 'primary');
    await expect(send).toHaveCSS('background-color', rgb(tokens.browser.color.accent[scheme]));
    await expect(send).toHaveCSS('text-decoration-line', 'none');
    await expect(card.locator('.tmt-ui-action[data-variant="primary"]')).toHaveCount(1);
    await expectOnePrimaryPerGroup(page, 'question card');
    const [goesBox, sendBox, afterBox] = [
      await card.locator('.question-goes-to').boundingBox(),
      await send.boundingBox(),
      await block.boundingBox(),
    ];
    expect(sendBox!.x + sendBox!.width).toBeLessThanOrEqual(afterBox!.x + afterBox!.width - 4);
    expect(sendBox!.x + sendBox!.width).toBeGreaterThan(afterBox!.x + afterBox!.width - 24);
    expect(goesBox!.x).toBeLessThan(sendBox!.x);

    // A keyboard-focused line draws its ring inside the band.
    await page.keyboard.press('Shift');
    await line.focus();
    await page.keyboard.press('Shift');
    await expect(line).toHaveCSS('outline-style', 'solid');
    const lineBox = await line.boundingBox();
    expect(lineBox!.x).toBeGreaterThanOrEqual(afterBox!.x);
    expect(lineBox!.x + lineBox!.width).toBeLessThanOrEqual(afterBox!.x + afterBox!.width);
    await page.mouse.move(0, 0);
    await page.screenshot({ path: capturePath(`question-card-v2-${width}-${scheme}.png`) });

    // The collapse works as before.
    await line.click();
    await expect(line).toHaveAttribute('aria-expanded', 'false');
    await expect(card.locator('.question-compact-controls')).toBeHidden();
    await line.click();

    // Answered: the check and the quiet state word, the recorded answer under the title, and the
    // card folds.
    await send.click();
    await expect(line.locator('.question-line-state')).toHaveText('Answered');
    await expect(line.locator('.lucide-check')).toHaveCount(1);
    await expect(line.locator('.question-line-answer')).toHaveText('Keep it');
    await expect(line.locator('.lucide-chevron-down')).toHaveCount(1);
    await expect(line.locator('.question-line-state')).toHaveCSS(
      'color',
      rgb(tokens.browser.color.secondary[scheme]),
    );
  });
}

test('both state words read at 4.5:1 on the sheet in light and dark', () => {
  for (const scheme of ['light', 'dark'] as const)
    for (const role of ['waiting', 'secondary'] as const)
      expect(
        contrast(tokens.browser.color[role][scheme], tokens.browser.surface.sheet[scheme]),
        `${role} ${scheme}`,
      ).toBeGreaterThanOrEqual(4.5);
});

test('a batch card keeps one heading, rows separated by a rule and one primary Send', async ({
  page,
}) => {
  await mount(page, 1440, 'light');
  await run(page, 'questionGroup', { count: 3 });
  const card = page.locator('.question-group-card');
  await card.locator('.question-line').first().click();
  await expect(card.locator('.question-compact-heading')).toContainText('3 of 3 waiting');
  // The heading names the agent once, so the rows carry no eyebrow.
  await expect(card.locator('.question-line-eyebrow')).toHaveCount(0);
  // Rows are separated by the same rule as the band's.
  const rows = card.locator('.question-compact-block > div');
  await expect(rows).toHaveCount(3);
  await expect(rows.nth(1)).toHaveCSS('border-top-width', '1px');
  await expect(card.locator('.question-compact-controls:not([hidden])')).toHaveCount(1);
  await expect(card.getByRole('button', { name: /^Send answers/ })).toHaveAttribute(
    'data-variant',
    'primary',
  );
  await expectOnePrimaryPerGroup(page, 'question batch card');
});
