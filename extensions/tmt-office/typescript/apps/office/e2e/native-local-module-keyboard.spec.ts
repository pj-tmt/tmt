import { writeFileSync } from 'node:fs';
import path from 'node:path';
import { expect, test } from '@playwright/test';
import type { Locator } from '@playwright/test';
import { runCli, withSandbox } from '../../../../../../typescript/test/support/cli-process.js';
import { installNativeOffice, unusedLoopbackPort } from './native-office-fixture.js';
import { savedWorld } from './native-world-state.js';
import type { WorldSnapshot } from '../src/world-map/world-port.js';
import { openKeyboardSelection, openOfficeDirectory } from './office-navigation.js';

test('module form keyboard actions and object repair share history and native persistence across desktop and narrow screens', async ({
  page,
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
    const initial: WorldSnapshot = await office(['layout', 'show']);
    const file = path.join(sandbox.root, 'keyboard-world.json');
    writeFileSync(file, JSON.stringify(initial.layout));
    await office([
      'layout',
      'apply',
      '--file',
      file,
      '--if-revision',
      '0',
      '--legacy-basis',
      initial.legacyBasis!,
    ]);
    const before = savedWorld(sandbox.database);
    let started = await office(['start', '--port', String(await unusedLoopbackPort())]);
    const activate = async (control: Locator) => {
      await expect(control).toBeVisible();
      await expect(control).toBeEnabled();
      await control.focus();
      await page.keyboard.press('Enter');
    };
    const button = (name: string) => page.getByRole('button', { name, exact: true });
    const nextWorldWrite = () =>
      page.waitForResponse(
        (response) =>
          new URL(response.url()).pathname === '/api/v1/local/world' &&
          response.request().method() === 'PUT'
      );
    const type = async (input: Locator, value: string) => {
      await expect(input).toBeVisible();
      await input.focus();
      await page.keyboard.press('ControlOrMeta+A');
      await page.keyboard.insertText(value);
    };
    const enter = async () => {
      await page.goto(started.url);
      await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
    };
    try {
      await page.setViewportSize({ width: 1536, height: 1024 });
      await enter();
      await expect(button('Add floor')).toHaveCount(0);
      await openOfficeDirectory(page);
      await page.getByText('Available spaces', { exact: true }).click();
      await activate(button('Area · column -1, row 0'));
      const form = page.getByRole('region', { name: 'New area' });
      await expect(form.getByRole('button', { name: 'Add area' })).toBeDisabled();
      await expect(form.getByText('Column -1, row 0', { exact: true })).toBeVisible();
      await type(form.getByRole('textbox', { name: 'Area name', exact: true }), 'Keyboard studio');
      expect(savedWorld(sandbox.database)).toEqual(before);
      const addedWrite = nextWorldWrite();
      await activate(form.getByRole('button', { name: 'Add area' }));
      expect((await addedWrite).status()).toBe(200);
      await expect(
        page.getByRole('region', { name: 'Layout changes' }).getByRole('status')
      ).toHaveText('All changes applied');
      const added = savedWorld(sandbox.database);
      expect(added.revision).toBe(before.revision + 1);
      await expect(button('Undo')).toBeEnabled();
      const undoneWrite = nextWorldWrite();
      await activate(button('Undo'));
      expect((await undoneWrite).status()).toBe(200);
      await expect(
        page.getByRole('region', { name: 'Layout changes' }).getByRole('status')
      ).toHaveText('All changes applied');
      const undone = savedWorld(sandbox.database);
      expect(undone.revision).toBe(added.revision + 1);
      expect(undone.layout).toBe(before.layout);
      await expect(button('Undo')).toBeDisabled();
      const redoneWrite = nextWorldWrite();
      await activate(button('Redo'));
      expect((await redoneWrite).status()).toBe(200);
      await page.screenshot({ path: info.outputPath('module-keyboard-desktop.png') });
      await expect(
        page.getByRole('region', { name: 'Layout changes' }).getByRole('status')
      ).toHaveText('All changes applied');
      const saved = savedWorld(sandbox.database);
      const layout: WorldSnapshot['layout'] = JSON.parse(saved.layout);
      expect(saved.revision).toBe(undone.revision + 1);
      expect(saved.layout).toBe(added.layout);
      expect(layout.map.version).toBe(8);
      if (layout.map.version === 1 || initial.layout.map.version === 1)
        throw new Error('Expected modules');
      expect(layout.map.modules).toHaveLength(6);
      const created = layout.map.modules.find((module) => module.area.name === 'Keyboard studio')!;
      expect(created.area).toMatchObject({
        name: 'Keyboard studio',
        binding: { type: 'personal', identityId: null },
      });
      expect(layout.map.modules.filter((module) => module !== created)).toEqual(
        [...initial.layout.map.modules].sort((a, b) => a.area.id.localeCompare(b.area.id))
      );
      expect(layout.objects).toEqual(initial.layout.objects);
      await page.goto('about:blank');
      await office(['stop']);
      started = await office(['start', '--port', String(await unusedLoopbackPort())]);
      await page.setViewportSize({ width: 390, height: 844 });
      await enter();
      expect(savedWorld(sandbox.database)).toEqual(saved);
      await openKeyboardSelection(page);
      const objects = page.getByRole('combobox', { name: 'Object', exact: true });
      await objects.focus();
      await objects.selectOption({ index: 1 });
      await expect(objects).not.toHaveValue('');
      const selectedId = await objects.inputValue();
      expect(layout.objects.find((object) => object.id === selectedId)?.surface.type).toBe('floor');
      await expect(page.getByRole('spinbutton', { name: 'X', exact: true })).toBeHidden();
      await activate(page.getByText('Precise placement', { exact: true }));
      await type(page.getByRole('spinbutton', { name: 'X', exact: true }), '-4000');
      const rejection = page.waitForResponse(
        (response) =>
          new URL(response.url()).pathname === '/api/v1/local/world' &&
          response.request().method() === 'PUT'
      );
      await activate(button('Apply coordinates'));
      const response = await rejection;
      expect(response.status()).toBe(400);
      expect(await response.json()).toMatchObject({
        error: 'WORLD_INVALID',
        issues: expect.arrayContaining([{ objectId: selectedId, reason: 'outsideFloor' }]),
      });
      await expect(page.getByRole('alert')).toBeVisible();
      expect(savedWorld(sandbox.database)).toEqual(saved);
      await activate(button('Select affected object'));
      await expect(objects).toHaveValue(selectedId);
      await expect(page.getByRole('spinbutton', { name: 'X', exact: true })).toHaveValue('-4000');
      await page.screenshot({ path: info.outputPath('module-keyboard-narrow-repair.png') });
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
        390
      );
      await activate(button('Undo'));
      await expect(
        page.getByRole('region', { name: 'Layout changes' }).getByRole('status')
      ).toHaveText('All changes applied');
      expect(savedWorld(sandbox.database)).toEqual(saved);
      expect((await office(['layout', 'show'])).layout).toEqual(layout);
    } finally {
      await office(['stop']);
      expect((await office(['status'])).service.running).toBe(false);
    }
  });
});
