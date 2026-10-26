import { expect, test } from '@playwright/test';
import { fileURLToPath } from 'node:url';

const fixturePath = `/@fs${fileURLToPath(new URL('../test/fixtures/topbar-ready', import.meta.url))}`;

for (const scenario of ['nativeReadiness', 'hostedReadiness', 'reactReadiness'] as const)
  test(`top bar readiness: ${scenario}`, async ({ page }) => {
    const errors: string[] = [];
    page.on('pageerror', (error) => errors.push(error.message));
    await page.goto(`${fixturePath}.html`);
    const result = await page.evaluate(
      async ({ name, path }) => {
        const fixture = await import(/* @vite-ignore */ path);
        return fixture[name]();
      },
      { name: scenario, path: `${fixturePath}.tsx` },
    );
    if (scenario === 'nativeReadiness')
      expect(result).toEqual({
        states: [false, true, true, false, true, false, false],
        barsBeforeAdoption: 0,
        action: 'Native action',
        missingRefused: true,
        clearedRefused: true,
        disposedRefused: true,
      });
    else if (scenario === 'hostedReadiness')
      expect(result).toEqual({
        before: false,
        installed: true,
        stale: false,
        cleared: false,
        disposed: false,
        readings: [false, true, false, true, false, false],
        disposalReadings: [false],
        action: 'Second action',
        clearedRefused: true,
      });
    else
      expect(result).toEqual({
        outside: false,
        absent: false,
        legacy: 'Legacy header',
        installed: true,
        action: 'React action',
        cleared: false,
        barsAfterClear: 0,
      });
    expect(errors).toEqual([]);
    expect(await page.locator('.tmt-ui-shell').count()).toBe(0);
  });
