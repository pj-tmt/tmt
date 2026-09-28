import { openKeyboardSelection, openMeetingRooms } from './office-navigation.js';
import { writeFileSync } from 'node:fs';
import path from 'node:path';
import Database from '../../../../../../typescript/test/support/sqlite-oracle.js';
import { expect, test } from '@playwright/test';
import { runCli, withSandbox } from '../../../../../../typescript/test/support/cli-process.js';
import { officeWorldFixture } from '../../../../../../typescript/test/support/office-world.js';
import { installNativeOffice, unusedLoopbackPort } from './native-office-fixture.js';
import { openOfficeDirectory, openOfficeObjects } from './office-navigation.js';
import type { WorldDocument, WorldObject } from '../src/world-map/world-contract.js';

test('meeting management and additive furniture share canonical resources without making layout cancellation destructive', async ({
  page,
}, info) => {
  test.setTimeout(120_000);
  await withSandbox(async (sandbox) => {
    const prefix = await installNativeOffice(sandbox);
    const cli = async (args: string[]) => {
      const result = await runCli(sandbox, [...args, '--json'], { deadlineMs: 30_000 });
      expect(result.status, result.stdout + result.stderr).toBe(0);
      return JSON.parse(result.stdout);
    };
    const office = (args: string[]) => cli(['office', '--prefix', prefix, ...args]);
    const alice = (await cli(['identity', 'create', 'Alice'])).identity;
    const design = (await cli(['room', 'create', 'Design'])).room;
    const initial = await office(['layout', 'show']);
    const base = officeWorldFixture().layout;
    const areaId = '70000000-0000-4000-8000-000000000001';
    const layout: WorldDocument = {
      ...base,
      objects: [],
      map: {
        ...base.map,
        areas: [
          ...base.map.areas,
          { id: areaId, name: 'Design area', binding: { type: 'meeting', roomId: design.id } },
        ],
        floor: [
          ...base.map.floor,
          ...Array.from({ length: 36 }, (_, y) => ({ y, start: 36, end: 72, areaId })),
        ],
        doors: [{ x: 36, y: 16, axis: 'vertical' }],
      },
    };
    const file = path.join(sandbox.root, 'meeting-tools.json');
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
    const before = await office(['layout', 'show']);
    const counts = () => {
      const db = new Database(sandbox.database, { readonly: true });
      try {
        return {
          boards: db.prepare('SELECT count(*) FROM office_whiteboards').pluck().get(),
          requests: db.prepare('SELECT count(*) FROM request_attempts').pluck().get(),
        };
      } finally {
        db.close();
      }
    };
    let started = await office(['start', '--port', String(await unusedLoopbackPort())]);
    const enter = async () => {
      await page.goto(started.url);
      await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
    };
    const begin = async () => {
      await openKeyboardSelection(page);
      await page.getByRole('combobox', { name: 'Area', exact: true }).selectOption(areaId);
    };
    const objects = page.getByRole('combobox', { name: 'Object', exact: true }).locator('option');
    const errors: string[] = [];
    page.on('pageerror', (error) => errors.push(error.message));
    try {
      await page.setViewportSize({ width: 1440, height: 1000 });
      await enter();
      await begin();
      await expect(
        page.getByRole('combobox', { name: 'Linked meeting room', exact: true })
      ).toBeHidden();
      await page.screenshot({ path: info.outputPath('meeting-inspector-simple.png') });
      await page.getByRole('button', { name: 'Add meeting set', exact: true }).click();
      await expect(objects).toHaveCount(10);
      await page.getByRole('button', { name: 'Undo', exact: true }).click();
      await expect(objects).toHaveCount(1);
      await page.getByRole('button', { name: 'Redo', exact: true }).click();
      await expect(objects).toHaveCount(10);

      await openMeetingRooms(page);
      const manager = page.getByRole('dialog', { name: 'Meeting rooms', exact: true });
      await manager
        .getByRole('combobox', { name: 'Meeting room', exact: true })
        .selectOption(design.id);
      await manager.getByRole('button', { name: 'Edit room', exact: true }).click();
      await manager.getByRole('checkbox', { name: 'Alice · offline', exact: true }).check();
      await manager.getByRole('button', { name: 'Save room', exact: true }).click();
      await expect(manager.getByText('Revision 2 · 1 members')).toBeVisible();
      await manager.getByRole('button', { name: 'Create room', exact: true }).click();
      await manager.getByRole('textbox', { name: 'Room name', exact: true }).fill('Planning');
      await manager.getByRole('button', { name: 'Close meeting rooms', exact: true }).click();
      await expect(objects).toHaveCount(10);
      await openMeetingRooms(page);
      await expect(manager.getByRole('textbox', { name: 'Room name', exact: true })).toHaveValue(
        'Planning'
      );
      await manager.getByRole('button', { name: 'Save room', exact: true }).click();
      await expect(manager.getByText('Revision 1 · 0 members')).toBeVisible();
      const planning = await manager
        .getByRole('combobox', { name: 'Meeting room', exact: true })
        .inputValue();
      await page.setViewportSize({ width: 390, height: 844 });
      await expect(manager.getByRole('button', { name: 'Edit room', exact: true })).toBeVisible();
      expect(await manager.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(
        true
      );
      await page.screenshot({ path: info.outputPath('meeting-manager-mobile.png') });
      await page.setViewportSize({ width: 1440, height: 1000 });
      await manager.getByRole('button', { name: 'Close meeting rooms', exact: true }).click();
      await expect(
        page.getByRole('combobox', { name: 'Linked meeting room', exact: true })
      ).toBeHidden();
      await page.getByText('Meeting link', { exact: true }).click();
      await page
        .getByRole('combobox', { name: 'Linked meeting room', exact: true })
        .selectOption(planning);
      await page.getByRole('button', { name: 'Undo', exact: true }).click();
      await expect(
        page.getByRole('combobox', { name: 'Linked meeting room', exact: true })
      ).toHaveValue(design.id);
      // The previous history entry is the meeting-set addition. Undo it explicitly;
      // the retired global Cancel action no longer owns an unsaved layout draft.
      await page.getByRole('button', { name: 'Undo', exact: true }).click();
      await page.getByRole('button', { name: 'Clear selection', exact: true }).click();
      await expect(
        page.getByRole('region', { name: 'Layout changes' }).getByRole('status')
      ).toHaveText('All changes applied');
      expect((await office(['layout', 'show'])).layout).toEqual(before.layout);
      expect((await cli(['room', 'show', design.id])).room).toMatchObject({
        revision: 2,
        memberIds: [alice.id],
      });
      expect((await cli(['room', 'show', planning])).room).toMatchObject({
        name: 'Planning',
        memberIds: [],
      });
      expect(counts()).toEqual({ boards: 0, requests: 0 });

      await begin();
      await page.getByRole('button', { name: 'Add meeting set', exact: true }).click();
      await expect(
        page.getByRole('region', { name: 'Layout changes' }).getByRole('status')
      ).toHaveText('All changes applied');
      const saved = await office(['layout', 'show']);
      expect(saved.revision).toBeGreaterThan(before.revision);
      expect(saved.layout.objects).toHaveLength(9);
      expect(
        saved.layout.objects
          .filter((object: WorldObject) => object.extension)
          .map((object: WorldObject) => object.extension!.binding)
      ).toEqual([
        { kind: 'whiteboard', documentId: design.id },
        { kind: 'office-board', roomId: design.id },
        { kind: 'office-broadcast' },
      ]);
      expect(counts()).toEqual({ boards: 0, requests: 0 });
      await openOfficeObjects(page);
      const objectActions = page.getByRole('navigation', { name: 'Office objects', exact: true });
      await expect(objectActions.getByText('No interactive objects in this area.')).toBeVisible();
      await expect(
        objectActions.getByRole('button', { name: 'Open whiteboard', exact: true })
      ).toHaveCount(0);
      await objectActions.getByText('Other areas · 1', { exact: true }).click();
      await expect(
        objectActions.getByRole('region', { name: 'Tools in Design area', exact: true })
      ).toBeVisible();
      await openOfficeDirectory(page);
      await page.getByRole('button', { name: /^Design area · meeting/ }).click();
      await expect(page.getByRole('heading', { name: 'Members · 1' })).toBeVisible();
      await page.screenshot({ path: info.outputPath('meeting-set-desktop.png') });
      await page.getByRole('button', { name: 'Open whiteboard', exact: true }).click();
      const board = page.getByRole('dialog', { name: 'Whiteboard', exact: true });
      await expect(board.getByText('New board · not saved')).toBeVisible();
      await board.getByRole('button', { name: 'Text', exact: true }).click();
      await board.getByLabel('Whiteboard drawing surface').click({ position: { x: 80, y: 80 } });
      await board
        .getByRole('textbox', { name: 'Text', exact: true })
        .fill('Design decisions survive removing the area.');
      await board.getByRole('button', { name: 'Apply text', exact: true }).click();
      await board.getByRole('button', { name: 'Save', exact: true }).click();
      await expect(board.getByText('Saved · revision 1')).toBeVisible();
      expect(counts()).toEqual({ boards: 1, requests: 0 });
      await board.getByRole('button', { name: 'Close whiteboard', exact: true }).click();

      // A spatial entry and the real CLI share one room-scoped board. Lose the
      // first response after native commit, then prove retry does not post twice.
      await page.getByRole('button', { name: 'Open discussion board', exact: true }).click();
      const discussion = page.getByRole('dialog', { name: 'Discussion board', exact: true });
      await expect(
        discussion.getByRole('button', { name: 'Room · Design', exact: true })
      ).toHaveAttribute('aria-pressed', 'true');
      await discussion.getByRole('button', { name: 'New post', exact: true }).click();
      await discussion.getByRole('textbox', { name: 'Title', exact: true }).fill('Design record');
      await discussion
        .getByRole('textbox', { name: 'Message', exact: true })
        .fill('Keep the room history when its area is removed.');
      const attempts: unknown[] = [];
      await page.route('**/api/v1/local/board/threads', async (route) => {
        attempts.push(route.request().postDataJSON());
        const response = await route.fetch();
        expect(response.ok()).toBe(true);
        if (attempts.length === 1) await route.abort('failed');
        else await route.fulfill({ response });
      });
      await discussion.getByRole('button', { name: 'Post as owner', exact: true }).click();
      await expect(discussion.getByRole('alert')).toContainText('draft is still here');
      const committed = await office(['board', 'list', '--room', design.id]);
      expect(committed.threads).toHaveLength(1);
      const discussionId = committed.threads[0].id;
      expect(committed.threads[0].category).toEqual({ kind: 'room', roomId: design.id });
      expect((await office(['board', 'list', '--general'])).threads).toEqual([]);
      expect((await office(['board', 'list', '--room', planning])).threads).toEqual([]);
      await discussion.getByRole('button', { name: 'General', exact: true }).click();
      await expect(
        discussion.getByRole('button', { name: 'Discard drafts and switch', exact: true })
      ).toBeDisabled();
      await discussion.getByRole('button', { name: 'Keep this discussion', exact: true }).click();
      await discussion.getByRole('button', { name: 'Post as owner', exact: true }).click();
      await expect(
        discussion.getByRole('heading', { name: 'Design record', exact: true })
      ).toBeVisible();
      expect(attempts).toHaveLength(2);
      expect(attempts[1]).toEqual(attempts[0]);
      expect((await office(['board', 'list', '--room', design.id])).threads).toHaveLength(1);
      await office([
        'board',
        'reply',
        discussionId,
        '--owner',
        '--body',
        'Confirmed through the native CLI.',
      ]);
      await discussion.getByRole('button', { name: 'Refresh board', exact: true }).click();
      await expect(
        discussion.getByText('Confirmed through the native CLI.', { exact: true })
      ).toBeVisible();
      await page.screenshot({ path: info.outputPath('meeting-discussion-native.png') });
      await discussion.getByRole('button', { name: 'Close discussion board', exact: true }).click();
      // Rebinding the spatial area changes its roster projection, not the
      // resource identities already mounted there. Save and reopen both tools.
      await begin();
      await expect(
        page.getByRole('combobox', { name: 'Linked meeting room', exact: true })
      ).toBeHidden();
      await page.getByText('Meeting link', { exact: true }).click();
      await page
        .getByRole('combobox', { name: 'Linked meeting room', exact: true })
        .selectOption(planning);
      await expect(
        page.getByRole('region', { name: 'Layout changes' }).getByRole('status')
      ).toHaveText('All changes applied');
      const rebound = await office(['layout', 'show']);
      expect(rebound.layout.objects).toEqual(saved.layout.objects);
      expect(
        rebound.layout.map.areas.find((area: { id: string }) => area.id === areaId).binding
      ).toEqual({ type: 'meeting', roomId: planning });
      expect((await cli(['room', 'show', design.id])).room.memberIds).toEqual([alice.id]);
      expect((await cli(['room', 'show', planning])).room.memberIds).toEqual([]);
      await openOfficeDirectory(page);
      await page.getByRole('button', { name: /^Design area · meeting/ }).click();
      await expect(page.getByRole('heading', { name: 'Members · 0' })).toBeVisible();
      await page.getByRole('button', { name: 'Open whiteboard', exact: true }).click();
      await expect(board.getByText('Saved · revision 1')).toBeVisible();
      await expect(board.getByText('1 elements · click to select')).toBeVisible();
      await board.getByRole('button', { name: 'Close whiteboard', exact: true }).click();
      await page.getByRole('button', { name: 'Open discussion board', exact: true }).click();
      await expect(
        discussion.getByRole('button', { name: 'Room · Design', exact: true })
      ).toHaveAttribute('aria-pressed', 'true');
      await expect(
        discussion.getByText('Confirmed through the native CLI.', { exact: true })
      ).toBeVisible();
      expect((await office(['board', 'list', '--room', planning])).threads).toEqual([]);
      expect(counts()).toEqual({ boards: 1, requests: 0 });
      await discussion.getByRole('button', { name: 'Close discussion board', exact: true }).click();
      await begin();
      await page.getByRole('button', { name: 'Remove area designation', exact: true }).click();
      await expect(
        page.getByRole('region', { name: 'Layout changes' }).getByRole('status')
      ).toHaveText('All changes applied');
      const detached = await office(['layout', 'show']);
      expect(detached.layout.objects).toEqual(saved.layout.objects);
      expect(detached.layout.map.areas).toHaveLength(1);
      expect((await cli(['room', 'show', design.id])).room.memberIds).toEqual([alice.id]);
      await page.goto('about:blank');
      await office(['stop']);
      started = await office(['start', '--port', String(await unusedLoopbackPort())]);
      await enter();
      expect(await office(['layout', 'show'])).toEqual(detached);
      await openOfficeObjects(page);
      const commonTools = page.getByRole('region', { name: 'Tools in Common floor', exact: true });
      await commonTools.getByRole('button', { name: 'Open whiteboard', exact: true }).click();
      await expect(board.getByText('Saved · revision 1')).toBeVisible();
      await expect(board.getByText('1 elements · click to select')).toBeVisible();
      await board.getByRole('button', { name: 'Close whiteboard', exact: true }).click();
      await commonTools.getByRole('button', { name: 'Open discussion board', exact: true }).click();
      await expect(
        discussion.getByRole('button', { name: 'Room · Design', exact: true })
      ).toHaveAttribute('aria-pressed', 'true');
      await expect(
        discussion.getByText('Confirmed through the native CLI.', { exact: true })
      ).toBeVisible();
      const persistedDiscussion = await office(['board', 'show', discussionId]);
      expect(persistedDiscussion.thread.category).toEqual({ kind: 'room', roomId: design.id });
      expect(persistedDiscussion.replies).toHaveLength(1);
      const db = new Database(sandbox.database, { readonly: true });
      try {
        expect(db.prepare('SELECT count(*) FROM office_board_entries').pluck().get()).toBe(2);
        expect(db.prepare('SELECT count(*) FROM office_board_operations').pluck().get()).toBe(2);
      } finally {
        db.close();
      }
      expect(counts()).toEqual({ boards: 1, requests: 0 });
      expect(errors).toEqual([]);
      await discussion.getByRole('button', { name: 'Close discussion board', exact: true }).click();
      await openMeetingRooms(page);
      await manager
        .getByRole('combobox', { name: 'Meeting room', exact: true })
        .selectOption(design.id);
      await manager.getByRole('button', { name: 'Retire room', exact: true }).click();
      await manager.getByRole('button', { name: 'Cancel retirement', exact: true }).click();
      expect((await cli(['room', 'show', design.id])).room.retired).toBe(false);
      const retireAttempts: unknown[] = [];
      await page.route(`**/api/v1/local/rooms/${design.id}/retire`, async (route) => {
        retireAttempts.push(route.request().postDataJSON());
        const response = await route.fetch();
        expect(response.ok()).toBe(true);
        if (retireAttempts.length === 1) await route.abort('failed');
        else await route.fulfill({ response });
      });
      await manager.getByRole('button', { name: 'Retire room', exact: true }).click();
      await manager.getByRole('button', { name: 'Confirm retirement', exact: true }).click();
      await expect(manager.getByRole('alert')).toContainText('Retirement was not confirmed');
      await manager.getByRole('button', { name: 'Confirm retirement', exact: true }).click();
      await expect(manager.getByRole('option', { name: 'Design', exact: true })).toHaveCount(0);
      expect(retireAttempts).toHaveLength(2);
      expect(retireAttempts[1]).toEqual(retireAttempts[0]);
      expect((await cli(['room', 'show', design.id])).room).toMatchObject({
        retired: true,
        memberIds: [alice.id],
      });
      await page.screenshot({ path: info.outputPath('meeting-retired-retained.png') });
      await manager.getByRole('button', { name: 'Close meeting rooms', exact: true }).click();
      await page.getByRole('button', { name: 'Clear selection', exact: true }).click();
      expect(await office(['layout', 'show'])).toEqual(detached);
      expect(counts()).toEqual({ boards: 1, requests: 0 });
      expect((await office(['board', 'list', '--room', design.id])).threads).toHaveLength(1);
      expect(await office(['board', 'show', discussionId])).toEqual(persistedDiscussion);
      await page.goto('about:blank');
      await office(['stop']);
      started = await office(['start', '--port', String(await unusedLoopbackPort())]);
      await enter();
      expect((await cli(['room', 'show', design.id])).room.retired).toBe(true);
      expect(await office(['layout', 'show'])).toEqual(detached);
      expect(await office(['board', 'show', discussionId])).toEqual(persistedDiscussion);
      await openOfficeObjects(page);
      await commonTools.getByRole('button', { name: 'Open whiteboard', exact: true }).click();
      await expect(board.getByText('Saved · revision 1')).toBeVisible();
      await board.getByRole('button', { name: 'Close whiteboard', exact: true }).click();
      await commonTools.getByRole('button', { name: 'Open discussion board', exact: true }).click();
      await expect(
        discussion.getByText('Confirmed through the native CLI.', { exact: true })
      ).toBeVisible();
      await discussion.getByRole('button', { name: 'Close discussion board', exact: true }).click();
      expect(errors).toEqual([]);
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
