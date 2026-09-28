import { randomUUID } from 'node:crypto';
import { writeFileSync } from 'node:fs';
import path from 'node:path';
import { expect, test } from '@playwright/test';
import { runCli, withSandbox } from '../../../../../../typescript/test/support/cli-process.js';
import { installNativeOffice, unusedLoopbackPort } from './native-office-fixture.js';
import { savedWorld } from './native-world-state.js';
import { fitWorldCoordinates } from './world-editor-gesture.js';
import type { WorldSnapshot } from '../src/world-map/world-port.js';
import type { MeetingRoom } from '../src/local/room-contract.js';
import type { WorldDocument } from '../src/world-map/world-contract.js';

test('unified area use preserves position and content through alignment, switching, creation, history and restart', async ({
  page,
}, info) => {
  await withSandbox(async (sandbox) => {
    const prefix = await installNativeOffice(sandbox);
    async function cli<T>(args: string[]): Promise<T> {
      const result = await runCli(sandbox, [...args, '--json'], { deadlineMs: 30_000 });
      expect(result.status, result.stdout + result.stderr).toBe(0);
      return JSON.parse(result.stdout) as T;
    }
    const office = <T>(args: string[]) => cli<T>(['office', '--prefix', prefix, ...args]);
    const fresh = await office<WorldSnapshot>(['layout', 'show']);
    expect(fresh.layout.map.version).toBe(8);
    if (fresh.layout.map.version === 1) throw new Error('Expected modular starter');
    const rooms: MeetingRoom[] = [];
    for (const name of ['Review', 'Planning', 'Focus']) {
      rooms.push((await cli<{ room: MeetingRoom }>(['room', 'create', name])).room);
    }
    const meetings = rooms.map((room, index) => ({
      area: {
        id: randomUUID(),
        name: room.name,
        binding: { type: 'meeting' as const, roomId: room.id },
      },
      slot: { type: 'meeting' as const, index },
      material: 'workshop' as const,
    }));
    const seed = {
      ...fresh.layout,
      objects: [
        ...fresh.layout.objects,
        ...fresh.layout.objects
          .filter((object) => object.extension?.definition === 'tmt-whiteboard')
          .map((object) => ({
            ...object,
            id: randomUUID(),
            placement: { ...object.placement, x: 144, y: 20 },
          })),
      ],
      map: {
        ...fresh.layout.map,
        version: 6 as const,
        modules: [...fresh.layout.map.modules, ...meetings],
      },
    };
    expect(seed.objects).toHaveLength(fresh.layout.objects.length + 1);
    const file = path.join(sandbox.root, 'retained-meetings.json');
    writeFileSync(file, JSON.stringify(seed));
    await office([
      'layout',
      'apply',
      '--file',
      file,
      '--if-revision',
      String(fresh.revision),
      '--legacy-basis',
      fresh.legacyBasis!,
    ]);
    const baseline = savedWorld(sandbox.database);
    const retained = JSON.parse(baseline.layout) as WorldDocument;
    if (retained.map.version === 1) throw new Error('Expected modular baseline');
    let started = await office<{ url: string }>([
      'start',
      '--port',
      String(await unusedLoopbackPort()),
    ]);
    const errors: string[] = [];
    page.on('pageerror', (error) => errors.push(error.message));
    const acknowledged = () =>
      page.waitForResponse(
        (response) =>
          response.request().method() === 'PUT' &&
          new URL(response.url()).pathname === '/api/v1/local/world' &&
          response.status() === 200
      );
    const layout = () => JSON.parse(savedWorld(sandbox.database).layout) as WorldDocument;
    try {
      await page.setViewportSize({ width: 1536, height: 1024 });
      await page.goto(started.url);
      await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
      let saved = acknowledged();
      await page.getByRole('button', { name: 'Use unified areas', exact: true }).click();
      await saved;
      const movedId = seed.objects.at(-1)!.id;
      const converted = {
        ...retained,
        objects: retained.objects.map((object) =>
          object.id === movedId ? { ...object, placement: { ...object.placement, x: 120 } } : object
        ),
        map: {
          ...retained.map,
          version: 8 as const,
          modules: retained.map.modules.map((module) => ({
            ...module,
            slot:
              module.slot.type === 'meeting'
                ? { type: 'office' as const, column: 2, row: module.slot.index }
                : module.slot,
          })),
        },
      };
      expect(layout()).toEqual(converted);
      expect(savedWorld(sandbox.database).revision).toBe(baseline.revision + 1);
      saved = acknowledged();
      await page.getByRole('button', { name: 'Undo', exact: true }).click();
      await saved;
      expect(layout()).toEqual(retained);
      saved = acknowledged();
      await page.getByRole('button', { name: 'Redo', exact: true }).click();
      await saved;
      expect(layout()).toEqual(converted);
      expect(savedWorld(sandbox.database).revision).toBe(baseline.revision + 3);
      await page.getByRole('button', { name: 'Fit office', exact: true }).click();
      await page.screenshot({ path: info.outputPath('unified-areas-desktop.png') });

      // Fixed lattice: 3 columns through X=192, rows -1..2 through Y=147,
      // plus the renderer's existing four/eight-unit framing margins.
      const point = await fitWorldCoordinates(page, { x: -4, y: -60, width: 200, height: 215 });
      const planning = point(168, 73.5);
      await page.mouse.click(planning.x, planning.y);
      await expect(page.getByRole('textbox', { name: 'Area name', exact: true })).toHaveValue(
        'Planning'
      );
      const selected = page.getByRole('region', { name: 'Selected area', exact: true });
      saved = acknowledged();
      await selected.getByRole('button', { name: 'Office', exact: true }).click();
      await saved;
      const officeUse = {
        ...converted,
        map: {
          ...converted.map,
          modules: converted.map.modules.map((module) =>
            module.area.id === meetings[1]!.area.id
              ? {
                  ...module,
                  area: {
                    ...module.area,
                    binding: { type: 'personal' as const, identityId: null },
                  },
                }
              : module
          ),
        },
      };
      expect(layout()).toEqual(officeUse);
      // Same real floor point selects the same platform after its use changes.
      await page.keyboard.press('Escape');
      await page.mouse.click(planning.x, planning.y);
      await expect(selected.getByRole('textbox', { name: 'Area name', exact: true })).toHaveValue(
        'Planning'
      );
      await selected.getByRole('button', { name: 'Meeting room', exact: true }).click();
      const link = selected.getByRole('region', { name: 'Create meeting space', exact: true });
      await link.getByText('Place an existing room', { exact: true }).click();
      await link
        .getByRole('combobox', { name: 'Existing room', exact: true })
        .selectOption(rooms[1]!.id);
      saved = acknowledged();
      await link.getByRole('button', { name: 'Place selected room', exact: true }).click();
      await saved;
      expect(layout()).toEqual(converted);
      saved = acknowledged();
      await page.getByRole('button', { name: 'Undo', exact: true }).click();
      await saved;
      expect(layout()).toEqual(officeUse);
      saved = acknowledged();
      await page.getByRole('button', { name: 'Redo', exact: true }).click();
      await saved;
      expect(layout()).toEqual(converted);
      await page.getByRole('button', { name: 'Review module removal', exact: true }).click();
      const removal = page.getByRole('dialog', { name: 'Remove Planning module', exact: true });
      saved = acknowledged();
      await removal.getByRole('button', { name: 'Remove module', exact: true }).click();
      await saved;
      const removed = {
        ...converted,
        map: {
          ...converted.map,
          modules: converted.map.modules.filter(
            (module) => module.area.id !== meetings[1]!.area.id
          ),
        },
      };
      expect(layout()).toEqual(removed);
      const retainedRooms = (await cli<{ rooms: MeetingRoom[] }>(['room', 'list'])).rooms;
      expect(retainedRooms).toHaveLength(rooms.length);
      expect(retainedRooms).toEqual(expect.arrayContaining(rooms));
      saved = acknowledged();
      await page.getByRole('button', { name: 'Undo', exact: true }).click();
      await saved;
      expect(layout()).toEqual(converted);
      saved = acknowledged();
      await page.getByRole('button', { name: 'Redo', exact: true }).click();
      await saved;
      expect(layout()).toEqual(removed);

      await page.keyboard.press('Escape');
      const ghost = planning;
      await page.mouse.move(ghost.x, ghost.y);
      await page.mouse.click(ghost.x, ghost.y);
      const areaCreation = page.getByRole('region', { name: 'New area', exact: true });
      await expect(areaCreation).toBeVisible();
      for (const width of [1536, 390]) {
        await page.setViewportSize({ width, height: 1024 });
        const gaps = await areaCreation.evaluate((element) => {
          const use = element.querySelector('.area-use-choices')!.getBoundingClientRect();
          const label = element.querySelector('label')!.getBoundingClientRect();
          const input = element.querySelector('input')!.getBoundingClientRect();
          const actions = element
            .querySelector('.office-expansion-actions')!
            .getBoundingClientRect();
          return { field: label.top - use.bottom, actions: actions.top - input.bottom };
        });
        expect(gaps.field).toBeGreaterThanOrEqual(16);
        expect(gaps.actions).toBeGreaterThanOrEqual(16);
        await page.screenshot({ path: info.outputPath(`area-office-spacing-${width}.png`) });
      }
      await page.setViewportSize({ width: 1536, height: 1024 });
      await areaCreation.getByRole('button', { name: 'Meeting room', exact: true }).click();
      const creation = areaCreation.getByRole('region', {
        name: 'Create meeting space',
        exact: true,
      });
      await expect(creation.getByRole('textbox', { name: 'Room name', exact: true })).toBeFocused();
      // Measure one layout frame: the anchored panel can move after its contents resize.
      const meetingGap = await creation.evaluate((element) => {
        const input = element.querySelector('input')!.getBoundingClientRect();
        const save = element.querySelector('button[type="submit"]')!.getBoundingClientRect();
        return save.top - input.bottom;
      });
      expect(meetingGap).toBeGreaterThanOrEqual(16);
      await page.screenshot({ path: info.outputPath('area-meeting-spacing.png') });
      await creation.getByRole('textbox', { name: 'Room name', exact: true }).fill('New planning');
      saved = acknowledged();
      await creation.getByRole('button', { name: 'Save room', exact: true }).click();
      await saved;
      await expect(creation).toHaveCount(0);
      const final = layout();
      if (final.map.version === 1) throw new Error('Expected modular areas');
      expect(final.objects.slice(0, converted.objects.length)).toEqual(converted.objects);
      expect(final.objects).toHaveLength(retained.objects.length + 9);
      expect(final.map.modules.find(({ area }) => area.name === 'New planning')).toMatchObject({
        slot: { type: 'office', column: 2, row: 1 },
        area: { binding: { type: 'meeting' } },
      });
      for (const module of removed.map.modules) expect(final.map.modules).toContainEqual(module);
      await page.setViewportSize({ width: 390, height: 844 });
      await page.getByRole('button', { name: 'Fit office', exact: true }).click();
      await page.screenshot({ path: info.outputPath('unified-areas-narrow.png') });
      const beforeRestart = savedWorld(sandbox.database);
      await page.goto('about:blank');
      await office(['stop']);
      started = await office<{ url: string }>([
        'start',
        '--port',
        String(await unusedLoopbackPort()),
      ]);
      await page.goto(started.url);
      await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
      expect(savedWorld(sandbox.database)).toEqual(beforeRestart);
      expect((await office<WorldSnapshot>(['layout', 'show'])).layout).toEqual(final);
      expect(errors).toEqual([]);
    } finally {
      await office(['stop']);
      expect(await office(['status'])).toMatchObject({ service: { running: false } });
    }
  });
});
