import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { expect, test } from '@playwright/test';
import { runCli, withSandbox } from '../../../../../../typescript/test/support/cli-process.js';
import { officeWorldFixture } from '../../../../../../typescript/test/support/office-world.js';
import type { ExtensionDefinition } from '../src/extensions/extension-contract.js';
import { installNativeOffice, unusedLoopbackPort } from './native-office-fixture.js';
import { openKeyboardSelection, openOfficeObjects } from './office-navigation.js';
import type { WorldDocument } from '../src/world-map/world-contract.js';

const definition = JSON.parse(
  readFileSync(new URL('../../../../contracts/link-extension-v1.json', import.meta.url), 'utf8')
) as ExtensionDefinition;

test('a persisted web object stays inert through restart and review until an explicit destination click', async ({
  page,
  context,
}, info) => {
  await withSandbox(async (sandbox) => {
    const prefix = await installNativeOffice(sandbox);
    const office = async (args: string[]) => {
      const result = await runCli(sandbox, ['office', '--prefix', prefix, ...args, '--json'], {
        deadlineMs: 30_000,
      });
      expect(result.status, result.stdout + result.stderr).toBe(0);
      return JSON.parse(result.stdout);
    };
    const initial = await office(['layout', 'show']);
    const destination = 'https://office-link.example.test/review?source=wall';
    const layout: WorldDocument = {
      ...officeWorldFixture().layout,
      objects: [
        {
          id: '30000000-0000-4000-8000-000000000001',
          kind: 'decoration',
          surface: { type: 'floor' },
          placement: { ...definition.appearance, x: 4, y: 4, rotation: 0 },
          extension: {
            definition: definition.id,
            binding: { kind: 'external-link', url: destination },
          },
        },
      ],
    };
    const file = path.join(sandbox.root, 'world-with-link.json');
    writeFileSync(file, JSON.stringify(layout));
    await office([
      'layout',
      'apply',
      '--file',
      file,
      '--if-revision',
      '0',
      '--legacy-basis',
      initial.legacyBasis,
    ]);
    const persisted = await office(['layout', 'show']);
    expect(persisted.layout).toEqual(layout);
    let requests = 0;
    // Observe every attempted destination request without contacting the network.
    await context.route('https://office-link.example.test/**', async (route) => {
      requests += 1;
      await route.fulfill({
        status: 200,
        contentType: 'text/html',
        body: '<title>Explicit link destination</title>',
      });
    });
    let started = await office(['start', '--port', String(await unusedLoopbackPort())]);
    const enter = async () => {
      await page.goto(started.url);
      await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
      await openOfficeObjects(page);
      await expect(
        page.getByRole('button', { name: 'Review web destination', exact: true })
      ).toBeVisible();
      expect(requests).toBe(0);
      expect(context.pages()).toHaveLength(1);
    };
    try {
      await enter();
      await openKeyboardSelection(page);
      await page
        .getByRole('combobox', { name: 'Object', exact: true })
        .selectOption(layout.objects[0]!.id);
      const action = page.locator('details.world-object-binding');
      await expect(action).not.toHaveAttribute('open');
      await expect(page.getByRole('button', { name: 'Delete', exact: true })).toBeVisible();
      await expect(action.getByLabel('Web destination', { exact: true })).toBeHidden();
      await page.getByText('Object action', { exact: true }).click();
      await expect(action.getByLabel('Web destination', { exact: true })).toHaveValue(destination);
      await page.getByText('Object action', { exact: true }).click();
      await page.screenshot({ path: info.outputPath('linked-object-compact-inspector.png') });
      expect(requests).toBe(0);
      await page.getByRole('button', { name: 'Clear selection', exact: true }).click();
      expect(await office(['layout', 'show'])).toEqual(persisted);
      await page.goto('about:blank');
      await office(['stop']);
      started = await office(['start', '--port', String(await unusedLoopbackPort())]);
      await enter();
      expect(await office(['layout', 'show'])).toEqual(persisted);
      await page.getByRole('button', { name: 'Review web destination', exact: true }).click();
      await expect(
        page.getByRole('heading', { name: 'Open an external website?', exact: true })
      ).toBeVisible();
      await expect(page.getByText(destination, { exact: true })).toBeVisible();
      const link = page.getByRole('link', { name: 'Open website ↗', exact: true });
      await expect(link).toHaveAttribute('rel', 'noopener noreferrer');
      expect(requests).toBe(0);
      expect(context.pages()).toHaveLength(1);
      await page.screenshot({ path: info.outputPath('native-link-review.png') });
      const opened = context.waitForEvent('page');
      await link.click();
      const target = await opened;
      await expect(target).toHaveTitle('Explicit link destination');
      expect(target.url()).toBe(destination);
      expect(await target.evaluate(() => window.opener)).toBeNull();
      expect(requests).toBe(1);
      await target.close();
      expect(await office(['layout', 'show'])).toEqual(persisted);
    } finally {
      try {
        if (!page.isClosed()) await page.goto('about:blank');
      } finally {
        await office(['stop']);
        expect((await office(['status'])).service.running).toBe(false);
      }
    }
  });
});
