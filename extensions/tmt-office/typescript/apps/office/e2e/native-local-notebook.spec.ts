import { existsSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { expect, test } from '@playwright/test';
import { runCli, withSandbox } from '../../../../../../typescript/test/support/cli-process.js';
import { officeWorldFixture } from '../../../../../../typescript/test/support/office-world.js';
import { installNativeOffice, unusedLoopbackPort } from './native-office-fixture.js';
import { openOfficeObjects } from './office-navigation.js';
import type { WorldSnapshot } from '../src/world-map/world-port.js';
import { openKeyboardSelection } from './office-navigation.js';

test.use({ actionTimeout: 10_000 });

test('owner binds CLI notes, reads exact inert content, refreshes and restarts without copying or deleting the source', async ({
  page,
  context,
}, info) => {
  await withSandbox(async (sandbox) => {
    const prefix = await installNativeOffice(sandbox);
    async function cli<T>(args: string[]): Promise<T> {
      const result = await runCli(sandbox, [...args, '--json'], { deadlineMs: 30_000 });
      expect(result.status, result.stdout + result.stderr).toBe(0);
      return JSON.parse(result.stdout);
    }
    const office = <T>(args: string[]) => cli<T>(['office', '--prefix', prefix, ...args]);
    const alice = (await cli<{ identity: { id: string } }>(['identity', 'create', 'Alice']))
      .identity;
    const initial = await office<WorldSnapshot>(['layout', 'show']);
    const layout = officeWorldFixture().layout;
    const input = path.join(sandbox.root, 'notebook-world.json');
    writeFileSync(input, JSON.stringify(layout));
    await office([
      'layout',
      'apply',
      '--file',
      input,
      '--if-revision',
      '0',
      '--legacy-basis',
      initial.legacyBasis!,
    ]);
    const expectedPath = path.join(sandbox.globalDir, 'notes', alice.id, 'notes.md');
    let started = await office<{ url: string }>([
      'start',
      '--port',
      String(await unusedLoopbackPort()),
    ]);
    let externalRequests = 0;
    await context.route('https://notebook.example.test/**', async (route) => {
      externalRequests++;
      await route.fulfill({ body: '' });
    });
    const open = async () => {
      await openOfficeObjects(page);
      await page.getByRole('button', { name: 'Read agent notebook', exact: true }).click();
    };
    const close = () => page.getByRole('button', { name: 'Close notebook', exact: true }).click();
    const content = page.getByLabel('Notebook content', { exact: true });
    try {
      await page.setViewportSize({ width: 1440, height: 1000 });
      await page.goto(started.url);
      await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
      await openKeyboardSelection(page);
      await page
        .getByRole('combobox', { name: 'Object', exact: true })
        .selectOption(layout.objects[0]!.id);
      await page.getByText('Object action', { exact: true }).click();
      await expect(page.getByRole('button', { name: 'Attach notebook to object' })).toBeDisabled();
      await page
        .getByRole('combobox', { name: 'Notebook owner', exact: true })
        .selectOption(alice.id);
      await expect(page.getByRole('combobox', { name: 'Notebook owner', exact: true })).toHaveValue(
        alice.id
      );
      await expect(page.getByRole('button', { name: 'Attach notebook to object' })).toBeEnabled();
      await page.getByRole('button', { name: 'Attach notebook to object' }).click();
      await expect(
        page.getByRole('region', { name: 'Layout changes' }).getByRole('status')
      ).toHaveText('All changes applied');
      const persisted = await office<WorldSnapshot>(['layout', 'show']);
      expect(persisted.layout.objects[0]).toEqual({
        ...layout.objects[0],
        extension: {
          definition: 'tmt-notebook',
          binding: { kind: 'notebook', identityId: alice.id },
        },
      });
      await open();
      await expect(page.getByRole('alert')).toContainText('No notebook exists yet');
      expect(existsSync(path.join(sandbox.globalDir, 'notes'))).toBe(false);
      const notes = await cli<{ path: string }>(['notes', 'path', '--identity', 'Alice']);
      expect(notes.path).toBe(expectedPath);
      const text =
        '# Architecture notebook\r\n\r\nOne source of truth. 🤖\n<img src="https://notebook.example.test/pixel" />\n<script>throw new Error("unsafe")</script>\n\ufeffEnd  ';
      writeFileSync(notes.path, text);
      const inode = statSync(notes.path).ino;
      await page.getByRole('button', { name: 'Refresh notebook' }).click();
      await expect(content).toHaveJSProperty('textContent', text);
      await expect(
        page.getByRole('dialog', { name: 'Notebook', exact: true }).locator('img, script, a')
      ).toHaveCount(0);
      await page.screenshot({ path: info.outputPath('notebook-desktop.png') });
      expect(externalRequests).toBe(0);
      expect(readFileSync(notes.path, 'utf8')).toBe(text);
      expect(statSync(notes.path).ino).toBe(inode);

      const updated = '# Updated by Alice\n\nRetained after restart.\n' + 'Long note '.repeat(100);
      writeFileSync(notes.path, updated);
      await expect(content).toHaveJSProperty('textContent', text);
      await page.getByRole('button', { name: 'Refresh notebook' }).click();
      await expect(content).toHaveJSProperty('textContent', updated);
      await close();
      await page.goto('about:blank');
      await office(['stop']);
      started = await office<{ url: string }>([
        'start',
        '--port',
        String(await unusedLoopbackPort()),
      ]);
      await page.setViewportSize({ width: 390, height: 844 });
      await page.goto(started.url);
      await open();
      await expect(content).toHaveJSProperty('textContent', updated);
      await expect(page.getByRole('button', { name: 'Refresh notebook' })).toBeVisible();
      expect(
        await page
          .getByRole('dialog', { name: 'Notebook', exact: true })
          .evaluate((node) => node.scrollWidth <= node.clientWidth)
      ).toBe(true);
      await page.screenshot({ path: info.outputPath('notebook-narrow.png') });
      expect(await office<WorldSnapshot>(['layout', 'show'])).toEqual(persisted);

      await cli(['rm', 'Alice', '--force']);
      const replacement = (await cli<{ identity: { id: string } }>(['identity', 'create', 'Alice']))
        .identity;
      expect(replacement.id).not.toBe(alice.id);
      await page.getByRole('button', { name: 'Refresh notebook' }).click();
      await expect(page.getByRole('alert')).toContainText('no longer active');
      await expect(content).toHaveCount(0);
      expect(readFileSync(notes.path, 'utf8')).toBe(updated);
      expect(existsSync(path.join(sandbox.globalDir, 'notes', replacement.id))).toBe(false);
      await close();
      await openKeyboardSelection(page);
      await page
        .getByRole('combobox', { name: 'Object', exact: true })
        .selectOption(layout.objects[0]!.id);
      await page.getByRole('button', { name: 'Delete', exact: true }).click();
      await expect(
        page.getByRole('region', { name: 'Layout changes' }).getByRole('status')
      ).toHaveText('All changes applied');
      expect((await office<WorldSnapshot>(['layout', 'show'])).layout.objects).toEqual(
        layout.objects.slice(1)
      );
      expect(readFileSync(notes.path, 'utf8')).toBe(updated);
      expect(statSync(notes.path).ino).toBe(inode);
      expect(externalRequests).toBe(0);
    } finally {
      try {
        if (!page.isClosed()) await page.goto('about:blank');
      } finally {
        await office(['stop']);
      }
      expect((await office<{ service: { running: boolean } }>(['status'])).service.running).toBe(
        false
      );
    }
  });
});
