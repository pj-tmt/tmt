import { expect, test } from '@playwright/test';
import { readFileSync } from 'node:fs';

const tokens = JSON.parse(
  readFileSync(new URL('../../tokens/tokens.json', import.meta.url), 'utf8'),
);
const ref = (value: string) => tokens.browser.ref[value.slice(1, -1)] as string;
const rgb = (hex: string) =>
  `rgb(${[1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16)).join(', ')})`;
const b = (role: string, theme: 'dark' | 'light') =>
  rgb(ref(tokens.browser['palette-b'].surface[role][theme]));

/** Paints a probe with a role inside an element built from `markup`, and reads it back. */
async function probe(page: import('@playwright/test').Page, markup: string, style: string) {
  return page.evaluate(
    ({ markup, style }) => {
      const host = document.createElement('div');
      host.innerHTML = markup;
      document.body.append(host);
      const target = host.querySelector('[data-probe]') as HTMLElement;
      target.setAttribute('style', style);
      const computed = getComputedStyle(target);
      const result = { background: computed.backgroundColor, radius: computed.borderTopLeftRadius };
      host.remove();
      return result;
    },
    { markup, style },
  );
}

for (const scheme of ['dark', 'light'] as const)
  test(`palette B applies only in its scope, on the system ${scheme} theme`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: scheme });
    await page.goto('/?frame=glyphs');
    const paper = 'background: var(--tmt-ui-surface-paper)';
    // Outside the scope the current values stay.
    expect((await probe(page, '<div data-probe></div>', paper)).background).toBe(
      rgb(tokens.browser.surface.paper[scheme]),
    );
    // Inside: the system theme, an explicit theme on the scope, and a light theme around it.
    expect(
      (await probe(page, '<div data-tmt-palette="b"><div data-probe></div></div>', paper))
        .background,
    ).toBe(b('paper', scheme));
    expect(
      (
        await probe(
          page,
          '<div data-tmt-palette="b" data-theme="dark"><div data-probe></div></div>',
          paper,
        )
      ).background,
    ).toBe(b('paper', 'dark'));
    expect(
      (
        await probe(
          page,
          '<div data-theme="light"><div data-tmt-palette="b"><div data-probe></div></div></div>',
          paper,
        )
      ).background,
    ).toBe(b('paper', 'light'));
  });

test('one knob sets every radius in the scope, and pills stay round only where they mean a dot', async ({
  page,
}) => {
  await page.goto('/?frame=glyphs');
  const radius = async (knob: string, role: string) =>
    (
      await probe(
        page,
        `<div data-tmt-palette="b" style="--tmt-ui-ref-radius: ${knob}"><div data-probe></div></div>`,
        `border-radius: var(--tmt-ui-${role})`,
      )
    ).radius;
  for (const [knob, control, surface] of [
    ['2px', '2px', '4px'],
    ['4px', '4px', '6px'],
  ])
    expect([
      await radius(knob!, 'radius-s'),
      await radius(knob!, 'radius'),
      await radius(knob!, 'radius-l'),
      await radius(knob!, 'radius-badge'),
    ]).toEqual([control, control, surface, control]);
  expect(await radius('2px', 'radius-pill')).toBe('999px');
});

test('the shadcn set follows the scope: its primary reads the accent primitive', async ({
  page,
}) => {
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.goto('/?frame=theme-b&theme=dark&radius=2');
  const send = page.getByRole('button', { name: 'Send', exact: true }).first();
  await expect(send).toHaveCSS('background-color', rgb(tokens.browser.ref['red-500']));
  await expect(send).toHaveCSS('border-top-left-radius', '2px');
  await page.getByRole('button', { name: '4 / 6' }).click();
  await expect(send).toHaveCSS('border-top-left-radius', '4px');
  await page.getByRole('button', { name: 'Current' }).click();
  await expect(send).toHaveCSS('border-top-left-radius', '8px');
  await page.getByRole('button', { name: 'Light', exact: true }).click();
  await expect(page.locator('main.theme-b')).toHaveCSS('background-color', b('paper', 'light'));
});
