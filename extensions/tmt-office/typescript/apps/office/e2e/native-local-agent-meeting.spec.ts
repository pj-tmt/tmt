import Database from '../../../../../../typescript/test/support/sqlite-oracle.js';
import { expect, test } from '@playwright/test';
import { runCli, withSandbox } from '../../../../../../typescript/test/support/cli-process.js';
import { installNativeOffice, unusedLoopbackPort } from './native-office-fixture.js';
import { openAgentDetails } from './office-navigation.js';

test('agent entry reviews and saves canonical membership without moving home or dispatching work', async ({
  page,
}, info) => {
  await withSandbox(async (sandbox) => {
    const prefix = await installNativeOffice(sandbox);
    const cli = async (args: string[]) => {
      const result = await runCli(sandbox, [...args, '--json'], { deadlineMs: 30_000 });
      expect(result.status, result.stdout + result.stderr).toBe(0);
      return JSON.parse(result.stdout);
    };
    const office = (args: string[]) => cli(['office', '--prefix', prefix, ...args]);
    const alice = (await cli(['identity', 'create', 'Alice'])).identity;
    const bob = (await cli(['identity', 'create', 'Bob'])).identity;
    const room = (await cli(['room', 'create', 'Design'])).room;
    const before = await office(['layout', 'show']);
    const members = () => {
      const database = new Database(sandbox.database, { readonly: true });
      try {
        expect(database.prepare('SELECT count(*) FROM request_attempts').pluck().get()).toBe(0);
        return database
          .prepare(
            'SELECT identity_id FROM office_meeting_members WHERE room_id = ? ORDER BY identity_id'
          )
          .pluck()
          .all(room.id);
      } finally {
        database.close();
      }
    };
    let started = await office(['start', '--port', String(await unusedLoopbackPort())]);
    const manager = page.getByRole('dialog', { name: 'Meeting rooms', exact: true });
    const enter = async (name: string) => {
      await openAgentDetails(page, name);
      await page.getByRole('tab', { name: 'Info', exact: true }).click();
      await page.getByRole('button', { name: 'Add to meeting…', exact: true }).click();
      await manager
        .getByRole('combobox', { name: 'Meeting room', exact: true })
        .selectOption(room.id);
    };
    try {
      await page.setViewportSize({ width: 1536, height: 1024 });
      await page.goto(started.url);
      await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
      await enter('Bob');
      await manager.getByRole('button', { name: 'Add Bob', exact: true }).click();
      expect(members()).toEqual([]);
      await manager.getByRole('button', { name: 'Save room', exact: true }).click();
      await expect(manager.getByRole('button', { name: 'Bob is already a member' })).toBeDisabled();
      expect(members()).toEqual([bob.id]);
      await manager.getByRole('button', { name: 'Close meeting rooms' }).click();
      await enter('Alice');
      await manager.getByRole('button', { name: 'Add Alice', exact: true }).click();
      await expect(manager.getByRole('checkbox', { name: /Alice/ })).toBeChecked();
      await expect(manager.getByRole('checkbox', { name: /Bob/ })).toBeChecked();
      // Closing the panel preserves this draft. A different agent cannot silently
      // replace it just because both entry points initially target "all rooms".
      await manager.getByRole('button', { name: 'Close meeting rooms' }).click();
      await openAgentDetails(page, 'Bob');
      await page.getByRole('tab', { name: 'Info', exact: true }).click();
      await page.getByRole('button', { name: 'Add to meeting…', exact: true }).click();
      await expect(
        manager.getByText('Finish or discard the current room draft before switching rooms.')
      ).toBeVisible();
      await expect(
        manager.getByText('Choose a room for Alice. Review its members before saving.')
      ).toBeVisible();
      await expect(manager.getByRole('checkbox', { name: /Alice/ })).toBeChecked();
      await page.screenshot({ path: info.outputPath('agent-meeting-review.png') });
      // Responsive HUD keeps the same draft and real controls inside the viewport.
      await page.setViewportSize({ width: 390, height: 844 });
      await expect(manager).toBeVisible();
      const panel = await manager.boundingBox();
      expect(panel).not.toBeNull();
      expect(panel!.x).toBeGreaterThanOrEqual(0);
      expect(panel!.x + panel!.width).toBeLessThanOrEqual(390);
      expect(await manager.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(
        true
      );
      await manager.getByRole('button', { name: 'Discard room draft' }).scrollIntoViewIfNeeded();
      await page.screenshot({ path: info.outputPath('agent-meeting-review-narrow.png') });
      await manager.getByRole('button', { name: 'Discard room draft' }).focus();
      await page.keyboard.press('Escape');
      await expect(manager).not.toBeVisible();
      await page.setViewportSize({ width: 1536, height: 1024 });
      await page.getByRole('button', { name: 'Add to meeting…', exact: true }).click();
      await expect(manager.getByRole('checkbox', { name: /Alice/ })).toBeChecked();
      await expect(manager.getByRole('checkbox', { name: /Bob/ })).toBeChecked();
      expect(members()).toEqual([bob.id]);
      await manager.getByRole('button', { name: 'Discard room draft' }).click();
      expect(members()).toEqual([bob.id]);
      await manager.getByRole('button', { name: 'Add Alice', exact: true }).click();
      await manager.getByRole('button', { name: 'Save room', exact: true }).click();
      await expect(
        manager.getByRole('button', { name: 'Alice is already a member' })
      ).toBeDisabled();
      expect(members()).toEqual([alice.id, bob.id].sort());
      expect((await cli(['room', 'show', room.id])).room).toMatchObject({
        revision: 3,
        memberIds: [alice.id, bob.id].sort(),
      });
      expect(await office(['layout', 'show'])).toEqual(before);
      await office(['stop']);
      started = await office(['start', '--port', String(await unusedLoopbackPort())]);
      await page.goto(started.url);
      await enter('Alice');
      await expect(
        manager.getByRole('button', { name: 'Alice is already a member' })
      ).toBeDisabled();
      expect(members()).toEqual([alice.id, bob.id].sort());
      expect(await office(['layout', 'show'])).toEqual(before);
    } finally {
      await office(['stop']);
    }
  });
});
