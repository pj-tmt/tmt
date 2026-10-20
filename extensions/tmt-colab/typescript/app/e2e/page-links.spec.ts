import { mkdir } from 'node:fs/promises';
import { expect, test, type Page } from '@playwright/test';

const fixture = '/test/page-link-browser.tsx';
const first = '20000000-0000-4000-8000-000000000001';
const second = '30000000-0000-4000-8000-000000000001';
const outside = 'https://outside.example/a/complete/path?query=visible&value=123#section';
const frame = (page: Page) => page.frameLocator('#page-link-fixture iframe');
const bar = (page: Page) => page.getByRole('dialog');

async function mount(page: Page, source: string) {
  await page.goto('/');
  await page.evaluate(async ({ fixture, source }) => (await import(fixture)).mount(source), {
    fixture,
    source,
  });
  await expect(frame(page).locator('body')).toBeVisible();
  await expect(page.locator('#page-link-fixture .status')).toContainText('Live');
}
async function forge(page: Page, href: string, overrides: Record<string, unknown> = {}) {
  const session = await page.context().newCDPSession(page);
  try {
    // Playwright's element evaluations grant a user gesture. Keep the entire forged-message
    // delivery in CDP without one, including the render-ID read and receipt acknowledgement.
    const { result } = await session.send('Runtime.evaluate', {
      expression: 'document.querySelector("#page-link-fixture iframe").dataset.renderId',
      userGesture: false,
    });
    const renderId = result.value as string;
    const received = session.send('Runtime.evaluate', {
      expression: `new Promise(resolve => {
        const received = event => {
          if (event.data?.type !== 'colab.render.link') return;
          window.removeEventListener('message', received);
          resolve(navigator.userActivation.isActive);
        };
        window.addEventListener('message', received);
      })`,
      awaitPromise: true,
      userGesture: false,
    });
    const { frameTree } = await session.send('Page.getFrameTree');
    const child = frameTree.childFrames![0].frame.id;
    const { executionContextId } = await session.send('Page.createIsolatedWorld', {
      frameId: child,
    });
    await session.send('Runtime.evaluate', {
      contextId: executionContextId,
      expression: `parent.postMessage(${JSON.stringify({ type: 'colab.render.link', renderId, href, newTab: false, ...overrides })}, '*')`,
      userGesture: false,
    });
    return (await received).result.value as boolean;
  } finally {
    await session.detach();
  }
}

for (const tail of ['', '#details']) {
  test(`real same-space short link opens in app without a prompt ${tail}`, async ({ page }) => {
    await mount(page, `<a href="/p/${second.slice(0, 8)}${tail}">Another page</a>`);
    await frame(page).getByRole('link', { name: 'Another page' }).click();
    await expect(frame(page).getByText('Second page', { exact: true })).toBeVisible();
    await expect(bar(page)).toHaveCount(0);
    await expect(page).toHaveURL(new RegExp(encodeURIComponent(`/pages/${second}`)));
    if (tail) {
      await expect(page).toHaveURL(/anchor=details/);
      await expect.poll(() => page.evaluate(() => scrollY)).toBeGreaterThan(500);
    }
  });
}

test('same-page fragments scroll without a navigation or confirmation', async ({ page }) => {
  await mount(
    page,
    '<a href="#target">Within this page</a><div style="height:1400px"></div><h2 id="target">Target</h2>',
  );
  const id = await page.locator('#page-link-fixture iframe').getAttribute('data-render-id');
  const url = page.url();
  await frame(page).getByRole('link').click();
  await expect.poll(() => page.evaluate(() => scrollY)).toBeGreaterThan(500);
  await expect(page).toHaveURL(url);
  await expect(page.locator('#page-link-fixture iframe')).toHaveAttribute('data-render-id', id!);
  await expect(bar(page)).toHaveCount(0);
});

test('a full owner link preserves its cosmetic anchor through in-app navigation', async ({
  page,
}) => {
  await page.goto('/');
  const href = `${new URL(page.url()).origin}/#space=10000000-0000-4000-8000-000000000001&path=${encodeURIComponent(`/pages/${second}`)}&anchor=details`;
  await mount(page, `<a href="${href.replaceAll('&', '&amp;')}">Owner page</a>`);
  await frame(page).getByRole('link').click();
  await expect(frame(page).getByText('Second page', { exact: true })).toBeVisible();
  await expect(page).toHaveURL(/anchor=details/);
  await expect(bar(page)).toHaveCount(0);
});

for (const activation of ['Enter', 'middle', 'Control+Enter']) {
  test(`link supports ${activation} without navigating the frame`, async ({ page, context }) => {
    await mount(page, `<a href="/p/${second.slice(0, 8)}">Another page</a>`);
    const link = frame(page).getByRole('link');
    if (activation === 'Enter') {
      await link.press('Enter');
      await expect(frame(page).getByText('Second page', { exact: true })).toBeVisible();
    } else {
      const opened = context.waitForEvent('page');
      if (activation === 'middle') await link.click({ button: 'middle' });
      else await link.press(activation);
      const destination = await opened;
      try {
        await expect(destination).toHaveURL(new RegExp(`/p/${second.slice(0, 8)}`));
      } finally {
        await destination.close();
      }
      await expect(page).toHaveURL(new RegExp(encodeURIComponent(`/pages/${first}`)));
    }
    await expect(bar(page)).toHaveCount(0);
  });
}

test('modified internal activation opens a new tab and permits only one navigation per render', async ({
  page,
  context,
}) => {
  await mount(page, `<a href="/p/${second.slice(0, 8)}">Another page</a>`);
  const opened = context.waitForEvent('page');
  await frame(page)
    .getByRole('link')
    .click({ modifiers: ['ControlOrMeta'] });
  const destination = await opened;
  try {
    await expect(destination).toHaveURL(new RegExp(`/p/${second.slice(0, 8)}`));
    await expect(bar(page)).toHaveCount(0);
    await expect(page).toHaveURL(new RegExp(encodeURIComponent(`/pages/${first}`)));
    await forge(page, `/p/${second.slice(0, 8)}`);
    await expect(bar(page)).toHaveCount(0);
    expect(context.pages()).toHaveLength(2);
  } finally {
    await destination.close();
  }
});

test('a forged current-frame request without activation prompts and does not navigate', async ({
  page,
}) => {
  await mount(page, '<p>Untrusted scripted request</p>');
  const session = await page.context().newCDPSession(page);
  try {
    await expect
      .poll(
        async () => {
          const { result } = await session.send('Runtime.evaluate', {
            expression: 'navigator.userActivation.isActive',
            userGesture: false,
          });
          return result.value;
        },
        { timeout: 10_000 },
      )
      .toBe(false);
  } finally {
    await session.detach();
  }
  const before = page.url();
  expect(await forge(page, `/p/${second.slice(0, 8)}#details`)).toBe(false);
  await expect(bar(page)).toHaveAccessibleName('Open another Colab page?');
  await expect(bar(page)).toContainText('This page asked to switch pages.');
  await expect(bar(page).locator('.page-link-url')).toHaveText(`/p/${second.slice(0, 8)}#details`);
  await expect(page).toHaveURL(before);
  await bar(page).getByRole('button', { name: 'Open', exact: true }).click();
  await expect(frame(page).getByText('Second page', { exact: true })).toBeVisible();
  await expect(page).toHaveURL(/anchor=details/);
});

test('external link always confirms its full frozen URL; cancel makes no request', async ({
  page,
  context,
}) => {
  const requests: string[] = [];
  await context.route('https://outside.example/**', async (route) => {
    requests.push(route.request().url());
    await route.fulfill({ body: '<h1>Destination</h1>', contentType: 'text/html' });
  });
  await mount(page, `<a href="${outside.replaceAll('&', '&amp;')}">Outside</a>`);
  await frame(page).getByRole('link').click();
  await expect(bar(page)).toHaveAccessibleName('Open this link in a new tab?');
  await expect(bar(page).locator('.page-link-url')).toHaveText(outside);
  await expect(bar(page)).toContainText('Outside Colab');
  await expect(bar(page).getByRole('button', { name: 'Open', exact: true })).toBeFocused();
  await forge(page, 'https://outside.example/replaced');
  await expect(bar(page).locator('.page-link-url')).toHaveText(outside);
  expect(requests).toEqual([]);
  await bar(page).getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(bar(page)).toHaveCount(0);
  expect(requests).toEqual([]);
  expect(context.pages()).toHaveLength(1);
});

test('Open uses noopener and noreferrer and clears the external bar', async ({ page, context }) => {
  let referer: string | undefined;
  await context.route('https://outside.example/**', async (route) => {
    referer = route.request().headers().referer;
    await route.fulfill({ body: '<h1>Destination</h1>', contentType: 'text/html' });
  });
  await mount(page, `<a href="${outside.replaceAll('&', '&amp;')}">Outside</a>`);
  await frame(page).getByRole('link').click();
  const opened = context.waitForEvent('page');
  await bar(page).getByRole('button', { name: 'Open', exact: true }).click();
  const destination = await opened;
  try {
    await expect(destination.getByRole('heading', { name: 'Destination' })).toBeVisible();
    await expect(destination).toHaveURL(outside);
    expect(await destination.evaluate(() => opener === null)).toBe(true);
    expect(referer).toBeUndefined();
    await expect(bar(page)).toHaveCount(0);
  } finally {
    await destination.close();
  }
});

test('Copy stays available without claiming blocked-popup detection', async ({ page, context }) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  await mount(page, `<a href="${outside.replaceAll('&', '&amp;')}">Outside</a>`);
  await frame(page).getByRole('link').click();
  await expect(bar(page)).toContainText('If no tab opens, copy the link instead.');
  await bar(page).getByRole('button', { name: 'Copy link', exact: true }).click();
  await expect(bar(page).getByRole('button', { name: 'Copied', exact: true })).toBeVisible();
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(outside);
  await expect(bar(page)).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(bar(page)).toHaveCount(0);
});

for (const scheme of [
  'javascript:alert(1)',
  'data:text/html,secret',
  'file:///tmp/secret',
  'colab-attachment:secret',
]) {
  test(`refused scheme ${scheme.split(':')[0]} leaves the frame ready`, async ({
    page,
    context,
  }) => {
    await mount(page, `<a href="${scheme}">Refused</a>`);
    await frame(page).getByRole('link').click();
    await expect(bar(page)).toHaveAccessibleName(
      'Pages can open https links and other Colab pages only.',
    );
    await expect(bar(page)).not.toContainText(scheme);
    await expect(bar(page).getByRole('button', { name: 'Close', exact: true })).toHaveAttribute(
      'data-variant',
      'primary',
    );
    expect(context.pages()).toHaveLength(1);
    await expect(page.locator('#page-link-fixture .status')).toContainText('Live');
  });
}

test('wrong render, extra fields, and non-frame messages cannot open a bar', async ({ page }) => {
  await mount(page, '<p>Ready</p>');
  await forge(page, outside, { renderId: 'stale' });
  await forge(page, outside, { extra: true });
  const renderId = await page.locator('#page-link-fixture iframe').getAttribute('data-render-id');
  await page.evaluate(
    (renderId) =>
      window.postMessage(
        { type: 'colab.render.link', renderId, href: 'https://outside.example/', newTab: false },
        '*',
      ),
    renderId,
  );
  await expect(bar(page)).toHaveCount(0);
});

test('scripted location navigation still tears the frame down', async ({ page }) => {
  await mount(page, '<button onclick="location=\'about:blank\'">Navigate directly</button>');
  await frame(page).getByRole('button').click();
  await expect(page.locator('#page-link-fixture iframe')).toHaveCount(0);
  await expect(
    page.getByText('The page navigated. Reopen it to continue.', { exact: true }),
  ).toBeVisible();
});

test('replacing the page clears a frozen confirmation from the old render', async ({ page }) => {
  await mount(page, `<a href="${outside}">Outside</a>`);
  await frame(page).getByRole('link').click();
  await expect(bar(page)).toBeVisible();
  await page.evaluate(async ({ fixture }) => (await import(fixture)).mount('<p>Replacement</p>'), {
    fixture,
  });
  await expect(frame(page).getByText('Replacement')).toBeVisible();
  await expect(bar(page)).toHaveCount(0);
});

for (const width of [1440, 390])
  for (const theme of ['light', 'dark']) {
    test(`external confirmation look ${width} ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await mount(page, `<a href="${outside.replaceAll('&', '&amp;')}">Outside</a>`);
      await page.evaluate((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      await frame(page).getByRole('link').click();
      await expect(bar(page).locator('.page-link-url')).toHaveText(outside);
      expect(await bar(page).evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);
      const directory = process.env.TMT_LINK_CAPTURE_DIR;
      if (directory) {
        await mkdir(directory, { recursive: true });
        await page.screenshot({ path: `${directory}/link-${width}-${theme}.png` });
      }
      await page.getByRole('heading', { name: 'First page', exact: true }).click();
      await expect(bar(page)).toHaveCount(0);
      await forge(page, 'data:text/html,private');
      await expect(bar(page)).toHaveAccessibleName(
        'Pages can open https links and other Colab pages only.',
      );
      if (directory) await page.screenshot({ path: `${directory}/refused-${width}-${theme}.png` });
    });
  }
