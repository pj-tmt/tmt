import { beginMeetingCreation, openKeyboardSelection } from './office-navigation.js';
import { writeFileSync } from 'node:fs';
import path from 'node:path';
import Database from '../../../../../../typescript/test/support/sqlite-oracle.js';
import { expect, test } from '@playwright/test';
import { runCli, withSandbox } from '../../../../../../typescript/test/support/cli-process.js';
import { installNativeOffice, unusedLoopbackPort } from './native-office-fixture.js';
import { savedWorld } from './native-world-state.js';
import { openOfficeDirectory } from './office-navigation.js';
import vectors from '../../../../../../contracts/office/modules-central-grid-vectors.json' with { type: 'json' };
import { decodeModuleMap } from '../src/world-map/module-contract.js';
import type { MeetingRoom } from '../src/local/room-contract.js';
import type { WorldSnapshot } from '../src/world-map/world-port.js';

test('world meeting entry saves canonical rooms separately from undoable spaces and membership', async ({
  page,
}, info) => {
  test.setTimeout(120_000);
  await withSandbox(async (sandbox) => {
    const prefix = await installNativeOffice(sandbox);
    async function cli<T>(args: string[]): Promise<T> {
      const result = await runCli(sandbox, [...args, '--json'], { deadlineMs: 30_000 });
      expect(result.status, result.stdout + result.stderr).toBe(0);
      return JSON.parse(result.stdout) as T;
    }
    const office = <T>(args: string[]) => cli<T>(['office', '--prefix', prefix, ...args]);
    const alice = await cli<{ identity: { id: string } }>(['identity', 'create', 'Alice']);
    const initial = await office<WorldSnapshot>(['layout', 'show']);
    const source = decodeModuleMap({ ...vectors.map, modules: vectors.map.modules.slice(0, 5) });
    const layout = { ...initial.layout, map: source };
    const file = path.join(sandbox.root, 'meeting-modules.json');
    writeFileSync(file, JSON.stringify(layout));
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
    const baseline = savedWorld(sandbox.database);
    const storedRooms = () => {
      const db = new Database(sandbox.database, { readonly: true });
      try {
        return db
          .prepare(
            'SELECT room_id AS id, name, revision FROM office_meeting_rooms ORDER BY room_id'
          )
          .all();
      } finally {
        db.close();
      }
    };
    const started = await office<{ url: string }>([
      'start',
      '--port',
      String(await unusedLoopbackPort()),
    ]);
    const errors: string[] = [];
    page.on('pageerror', (error) => errors.push(error.message));
    const nextWorldWrite = () =>
      page.waitForResponse(
        (response) =>
          new URL(response.url()).pathname === '/api/v1/local/world' &&
          response.request().method() === 'PUT'
      );
    try {
      await page.setViewportSize({ width: 1536, height: 1024 });
      await page.goto(started.url);
      await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
      await page.getByRole('button', { name: 'Fit office', exact: true }).click();
      // The hologram is canvas content; the HUD supplies keyboard access.
      // Pointer pan/pinch/click behavior is covered by native-local-skybridges.
      await expect(page.locator('.meeting-entry')).toHaveCount(0);
      await page.screenshot({ path: info.outputPath('meeting-empty-wing.png') });
      expect(storedRooms()).toEqual([]);
      await beginMeetingCreation(page);
      const creation = page.getByRole('region', { name: 'Create meeting space', exact: true });
      const name = creation.getByRole('textbox', { name: 'Room name', exact: true });
      await expect(name).toBeFocused();
      await name.fill('Design review');
      await page.screenshot({ path: info.outputPath('meeting-name-desktop.png') });
      await page.setViewportSize({ width: 390, height: 844 });
      await page.getByRole('button', { name: 'Fit office', exact: true }).click();
      const card = await creation.boundingBox();
      expect(card!.x).toBeGreaterThanOrEqual(0);
      expect(card!.x + card!.width).toBeLessThanOrEqual(390);
      expect(card!.y).toBeGreaterThanOrEqual(0);
      expect(card!.y + card!.height).toBeLessThanOrEqual(844);
      await page.screenshot({ path: info.outputPath('meeting-name-narrow.png') });
      const initialPlacement = nextWorldWrite();
      await creation.getByRole('button', { name: 'Save room', exact: true }).click();
      expect((await initialPlacement).status()).toBe(200);
      await expect(creation).toHaveCount(0);
      const rooms = await cli<{ rooms: MeetingRoom[] }>(['room', 'list']);
      expect(rooms.rooms).toHaveLength(1);
      const room = rooms.rooms[0]!;
      expect(room.name).toBe('Design review');
      expect(storedRooms()).toEqual([{ id: room.id, name: room.name, revision: 1 }]);
      await expect(
        page.getByRole('region', { name: 'Layout changes' }).getByRole('status')
      ).toHaveText('All changes applied');
      await openKeyboardSelection(page);
      const areas = page.getByRole('combobox', { name: 'Area', exact: true }).locator('option');
      await expect(areas).toHaveCount(7); // Common floor plus five original modules and the new room.
      await page.getByRole('button', { name: 'Undo', exact: true }).click();
      await expect(areas).toHaveCount(6);
      await page.getByRole('button', { name: 'Redo', exact: true }).click();
      await expect(areas).toHaveCount(7);
      await page.getByRole('button', { name: 'Undo', exact: true }).click();
      await expect(
        page.getByRole('region', { name: 'Layout changes' }).getByRole('status')
      ).toHaveText('All changes applied');
      expect(JSON.parse(savedWorld(sandbox.database).layout)).toEqual(JSON.parse(baseline.layout));
      expect(storedRooms()).toHaveLength(1);
      await page.setViewportSize({ width: 1536, height: 1024 });
      await beginMeetingCreation(page);
      await creation.getByText('Place an existing room', { exact: true }).click();
      await creation
        .getByRole('combobox', { name: 'Existing room', exact: true })
        .selectOption(room.id);
      const restoredPlacement = nextWorldWrite();
      await creation.getByRole('button', { name: 'Place selected room', exact: true }).click();
      expect((await restoredPlacement).status()).toBe(200);
      await expect(creation).toHaveCount(0);
      await expect(
        page.getByRole('region', { name: 'Layout changes' }).getByRole('status')
      ).toHaveText('All changes applied');
      const saved = await office<WorldSnapshot>(['layout', 'show']);
      expect(saved.layout.map.version).toBe(4);
      if (saved.layout.map.version === 1) throw new Error('Expected modular source');
      const module = saved.layout.map.modules.find((module) => module.slot.type === 'meeting')!;
      expect(module.slot).toEqual({ type: 'meeting', index: 0 });
      expect(module.area.binding).toEqual({ type: 'meeting', roomId: room.id });
      expect(saved.layout.objects.slice(0, layout.objects.length)).toEqual(layout.objects);
      expect(saved.layout.objects.length).toBe(layout.objects.length + 9);
      expect(JSON.parse(savedWorld(sandbox.database).layout)).toEqual(saved.layout);
      await page.getByRole('button', { name: 'Fit office', exact: true }).click();
      await page.screenshot({ path: info.outputPath('meeting-furnished.png') });
      await page.getByRole('button', { name: 'Zoom in', exact: true }).click();
      await page.getByRole('button', { name: 'Zoom in', exact: true }).click();
      await page.screenshot({ path: info.outputPath('meeting-wall-detail.png') });
      await page.getByRole('button', { name: 'Fit office', exact: true }).click();
      await openOfficeDirectory(page);
      await page
        .getByRole('button', { name: 'Design review · meeting Design review', exact: true })
        .click();
      await page.getByRole('button', { name: 'Manage members', exact: true }).click();
      const manager = page.getByRole('dialog', { name: 'Meeting rooms', exact: true });
      await manager.getByRole('button', { name: 'Edit room', exact: true }).click();
      await manager.getByRole('checkbox', { name: 'Alice · offline', exact: true }).check();
      await manager.getByRole('button', { name: 'Save room', exact: true }).click();
      await expect(manager.getByText('Revision 2 · 1 members')).toBeVisible();
      const updated = await cli<{ room: MeetingRoom }>(['room', 'show', room.id]);
      expect(updated.room.memberIds).toEqual([alice.identity.id]);
      expect(JSON.parse(savedWorld(sandbox.database).layout)).toEqual(saved.layout);
      await manager.getByRole('button', { name: 'Close meeting rooms', exact: true }).click();
      await openKeyboardSelection(page);
      await page.getByRole('combobox', { name: 'Area', exact: true }).selectOption(module.area.id);
      await page.getByRole('button', { name: 'Review module removal', exact: true }).click();
      const removal = page.getByRole('dialog', {
        name: 'Remove Design review module',
        exact: true,
      });
      await expect(
        removal.getByRole('button', { name: 'Remove module', exact: true })
      ).toBeDisabled();
      await removal.getByRole('button', { name: 'Cancel removal', exact: true }).click();
      await page.goto('about:blank');
      await page.goto(started.url);
      await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
      expect(JSON.parse(savedWorld(sandbox.database).layout)).toEqual(saved.layout);
      expect(storedRooms()).toHaveLength(1);
      expect(errors).toEqual([]);
      // Compact conversion uses the same serialized auto-apply fence as ordinary edits.
      // It must not mutate canonical discussions, and Undo must restore the saved layout.
      const roomBaseline = storedRooms();
      const compact = page.getByRole('button', { name: 'Preview compact layout', exact: true });
      await compact.click();
      await expect(compact).toHaveCount(0);
      await page.getByRole('button', { name: 'Undo', exact: true }).click();
      await expect(compact).toBeVisible();
      await expect(
        page.getByRole('region', { name: 'Layout changes' }).getByRole('status')
      ).toHaveText('All changes applied');
      expect(JSON.parse(savedWorld(sandbox.database).layout)).toEqual(saved.layout);
      await compact.click();
      await expect(
        page.getByRole('region', { name: 'Layout changes' }).getByRole('status')
      ).toHaveText('All changes applied');
      const converted = await office<WorldSnapshot>(['layout', 'show']);
      expect(converted.layout.map).toEqual({ ...saved.layout.map, version: 5 });
      expect(converted.layout.objects).toEqual(saved.layout.objects);
      expect(storedRooms()).toEqual(roomBaseline);
      await page.goto('about:blank');
      await page.goto(started.url);
      await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
      await page.getByRole('button', { name: 'Fit office', exact: true }).click();
      await page.screenshot({ path: info.outputPath('compact-world.png') });
      expect(JSON.parse(savedWorld(sandbox.database).layout)).toEqual(converted.layout);
      await beginMeetingCreation(page);
      await creation.getByRole('textbox', { name: 'Room name', exact: true }).fill('Planning');
      const adjacentPlacement = nextWorldWrite();
      await creation.getByRole('button', { name: 'Save room', exact: true }).click();
      expect((await adjacentPlacement).status()).toBe(200);
      await expect(
        page.getByRole('region', { name: 'Layout changes' }).getByRole('status')
      ).toHaveText('All changes applied');
      const adjacent = await office<WorldSnapshot>(['layout', 'show']);
      if (adjacent.layout.map.version === 1) throw new Error('Expected modular source');
      expect(
        adjacent.layout.map.modules
          .flatMap((entry) => (entry.slot.type === 'meeting' ? [entry.slot.index] : []))
          .sort((a, b) => a - b)
      ).toEqual([0, 1]);
      expect(adjacent.layout.objects.slice(0, converted.layout.objects.length)).toEqual(
        converted.layout.objects
      );
      expect(adjacent.layout.objects).toHaveLength(converted.layout.objects.length + 9);
      await page.getByRole('button', { name: 'Fit office', exact: true }).click();
      await page.screenshot({ path: info.outputPath('compact-adjacent-meetings.png') });
      expect(errors).toEqual([]);
    } finally {
      await office(['stop']);
    }
  });
});
