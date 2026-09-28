import { mapGeometry } from '../src/world-map/map-source.js';
import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import Database from '../../../../../../typescript/test/support/sqlite-oracle.js';
import { expect, test } from '@playwright/test';
import { runCli, withSandbox } from '../../../../../../typescript/test/support/cli-process.js';
import { officeWorldFixture } from '../../../../../../typescript/test/support/office-world.js';
import { installNativeOffice, unusedLoopbackPort } from './native-office-fixture.js';
import { openKeyboardSelection, openOfficeDirectory } from './office-navigation.js';
import { savedWorld } from './native-world-state.js';
import type { WorldDocument } from '../src/world-map/world-contract.js';
import type { WorldSnapshot, WorldWrite } from '../src/world-map/world-port.js';

function retainedIdentityState(databasePath: string) {
  const db = new Database(databasePath, { readonly: true });
  try {
    return {
      identities: db.prepare('SELECT * FROM identities ORDER BY id').all(),
      profiles: db.prepare('SELECT * FROM office_local_profiles ORDER BY identity_id').all(),
      statuses: db.prepare('SELECT * FROM identity_status ORDER BY identity_id').all(),
      exchanges: db.prepare('SELECT * FROM request_attempts ORDER BY request_id').all(),
    };
  } finally {
    db.close();
  }
}

function expectStoredWorld(databasePath: string, snapshot: WorldSnapshot) {
  const row = savedWorld(databasePath);
  expect({ revision: row.revision, layout: JSON.parse(row.layout) }).toEqual({
    revision: snapshot.revision,
    layout: snapshot.layout,
  });
}

test('repairs retained personal and Lobby designations without deleting identity content or floor objects', async ({
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
    await cli(['identity', 'create', 'Sender']);
    const notes = await cli(['notes', 'path', '--identity', 'Alice']);
    expect(notes.path).toBe(path.join(sandbox.globalDir, 'notes', alice.id, 'notes.md'));
    const noteText = '# Retained work\nArea removal does not own identity notes.\n';
    writeFileSync(notes.path, noteText);
    await cli(['identity', 'status', 'set', 'Reviewing the office', '--identity', 'Alice']);
    const profile = (await office(['profile', 'show', '--local', '--identity', 'Alice'])).profile;
    const profileFile = path.join(sandbox.root, 'profile.json');
    writeFileSync(profileFile, JSON.stringify({ ...profile, description: 'Keep this profile.' }));
    await office([
      'profile',
      'apply',
      '--local',
      '--identity',
      'Alice',
      '--file',
      profileFile,
      '--if-revision',
      '0',
    ]);
    const request = await cli([
      'talk',
      'Alice',
      'Keep this exchange.',
      '--identity',
      'Sender',
      '--inbox',
      '--detach',
    ]);
    const incoming = await cli([
      'x',
      'show',
      request.requestId,
      '--incoming',
      '--identity',
      'Alice',
    ]);
    await cli([
      'reply',
      request.requestId,
      '--receipt',
      incoming.exchange.reply.receipt,
      '--message',
      'Retained reply.',
    ]);
    const retained = retainedIdentityState(sandbox.database);
    expect(retained.exchanges).toHaveLength(1);
    const initial: WorldSnapshot = await office(['layout', 'show']);
    const base = officeWorldFixture().layout;
    const lobbyId = base.map.primaryLobbyId;
    const studioId = 'a0000000-0000-4000-8000-000000000001';
    const layout: WorldDocument = {
      ...base,
      map: {
        ...base.map,
        areas: [
          ...base.map.areas,
          { id: studioId, name: 'Studio', binding: { type: 'personal', identityId: null } },
        ],
        floor: Array.from({ length: 36 }, (_, y) => [
          { y, start: 0, end: 36, areaId: studioId },
          { y, start: 36, end: 72, areaId: lobbyId },
        ]).flat(),
        doors: [{ x: 36, y: 16, axis: 'vertical' }],
      },
    };
    const file = path.join(sandbox.root, 'topology.json');
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
    let started = await office(['start', '--port', String(await unusedLoopbackPort())]);
    const enter = async () => {
      await page.goto(started.url);
      await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
    };
    const begin = () => openKeyboardSelection(page);
    const save = async () => {
      await expect(
        page.getByRole('region', { name: 'Layout changes' }).getByRole('status')
      ).toHaveText('All changes applied');
      return (await office(['layout', 'show'])) as WorldSnapshot;
    };
    const area = page.getByRole('combobox', { name: 'Area', exact: true });
    const remove = page.getByRole('button', { name: 'Remove area designation', exact: true });
    try {
      await page.setViewportSize({ width: 1440, height: 1000 });
      await enter();
      await begin();
      await expect(page.getByRole('button', { name: 'Create area', exact: true })).toHaveCount(0);
      await area.selectOption(studioId);
      await page.getByRole('combobox', { name: 'Resident', exact: true }).selectOption(alice.id);
      const assigned = await save();
      expect(mapGeometry(assigned.layout.map).areas).toContainEqual({
        id: studioId,
        name: 'Studio',
        binding: { type: 'personal', identityId: alice.id },
      });
      expect(
        mapGeometry(assigned.layout.map).floor.filter((span) => span.areaId === studioId)
      ).toEqual(Array.from({ length: 36 }, (_, y) => ({ y, start: 0, end: 36, areaId: studioId })));
      expect(assigned.layout.objects).toEqual(layout.objects);
      await openOfficeDirectory(page);
      await expect(
        page.getByRole('button', { name: 'Alice · Offline Studio', exact: true })
      ).toBeVisible();
      await page.keyboard.press('Escape');
      await begin();
      await area.selectOption(studioId);
      const impact = page.getByRole('region', { name: 'Area removal preview' });
      await impact.getByText('1 occupants affected', { exact: true }).click();
      await expect(impact.getByText('Alice', { exact: true })).toBeVisible();
      await impact.getByText('1 objects kept in place', { exact: true }).click();
      await expect(
        impact.getByText(`desk · ${layout.objects[0]!.id}`, { exact: true })
      ).toBeVisible();
      await page.screenshot({ path: info.outputPath('personal-area-removal-preview.png') });
      await remove.click();
      const autoDetached = await save();
      expect(mapGeometry(autoDetached.layout.map).areas).not.toContainEqual(
        expect.objectContaining({ id: studioId })
      );
      expect(autoDetached.layout.objects).toEqual(assigned.layout.objects);
      await page.getByRole('button', { name: 'Undo', exact: true }).click();
      await expect(page.getByRole('combobox', { name: 'Resident', exact: true })).toHaveValue(
        alice.id
      );
      await page.getByRole('button', { name: 'Redo', exact: true }).click();
      await page.getByRole('button', { name: 'Undo', exact: true }).click();
      const restoredAssigned = await save();
      expect((await office(['layout', 'show'])).layout).toEqual(assigned.layout);
      expectStoredWorld(sandbox.database, restoredAssigned);
      await begin();
      await area.selectOption(studioId);
      await remove.click();
      const detached = await save();
      const detachedLayout = {
        ...layout,
        map: {
          ...layout.map,
          areas: base.map.areas,
          floor: mapGeometry(layout.map).floor.map((span) =>
            span.areaId === studioId ? { ...span, areaId: null } : span
          ),
        },
      };
      expect(detached.layout).toEqual(detachedLayout);
      expectStoredWorld(sandbox.database, detached);
      await openOfficeDirectory(page);
      await expect(
        page.getByRole('button', { name: 'Alice · Offline Unassigned · Lobby', exact: true })
      ).toBeVisible();
      await page.screenshot({ path: info.outputPath('personal-area-removed.png') });
      await page.keyboard.press('Escape');
      // Retained multi-Lobby input comes from the native layout API, not a
      // resurrected area-painting UI. The browser must repair it explicitly.
      const replacementId = 'a0000000-0000-4000-8000-000000000002';
      const multiLobby = {
        ...detachedLayout,
        map: {
          ...detachedLayout.map,
          areas: [
            ...base.map.areas,
            { id: replacementId, name: 'New Lobby', binding: { type: 'lobby' } },
          ],
          floor: detachedLayout.map.floor.map((span) =>
            span.areaId === null ? { ...span, areaId: replacementId } : span
          ),
        },
      };
      writeFileSync(file, JSON.stringify(multiLobby));
      await office(['layout', 'apply', '--file', file, '--if-revision', String(detached.revision)]);
      await page.goto('about:blank');
      await enter();
      await begin();
      await area.selectOption(lobbyId);
      await expect(remove).toBeDisabled();
      await page
        .getByRole('combobox', { name: 'Replacement Lobby', exact: true })
        .selectOption(replacementId);
      await page.setViewportSize({ width: 430, height: 932 });
      await page.getByRole('button', { name: 'Fit office', exact: true }).click();
      await remove.scrollIntoViewIfNeeded();
      await page.screenshot({ path: info.outputPath('replacement-lobby-narrow.png') });
      await remove.click();
      const replaced = await save();
      expect(replaced.layout.map.primaryLobbyId).toBe(replacementId);
      expect(mapGeometry(replaced.layout.map).areas).toEqual([
        { id: replacementId, name: 'New Lobby', binding: { type: 'lobby' } },
      ]);
      expect(mapGeometry(replaced.layout.map).floor).toEqual(
        Array.from({ length: 36 }, (_, y) => [
          { y, start: 0, end: 36, areaId: replacementId },
          { y, start: 36, end: 72, areaId: null },
        ]).flat()
      );
      expect(replaced.layout.objects).toEqual(layout.objects);
      expect(mapGeometry(replaced.layout.map).doors).toEqual(mapGeometry(layout.map).doors);
      expectStoredWorld(sandbox.database, replaced);
      expect(retainedIdentityState(sandbox.database)).toEqual(retained);
      expect(readFileSync(notes.path, 'utf8')).toBe(noteText);
      await page.goto('about:blank');
      await office(['stop']);
      started = await office(['start', '--port', String(await unusedLoopbackPort())]);
      await enter();
      await openOfficeDirectory(page);
      await expect(
        page.getByRole('button', { name: 'Alice · Offline Unassigned · New Lobby', exact: true })
      ).toBeVisible();
      expect(await office(['layout', 'show'])).toEqual(replaced);
      expectStoredWorld(sandbox.database, replaced);
      expect(await cli(['result', request.requestId])).toMatchObject({
        status: 'completed',
        response: 'Retained reply.',
      });
      expect(retainedIdentityState(sandbox.database)).toEqual(retained);
      expect(readFileSync(notes.path, 'utf8')).toBe(noteText);
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

test('a real external world write rejects stale browser auto-apply without rebasing local changes or partially committing them', async ({
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
    const file = path.join(sandbox.root, 'conflicting-layout.json');
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
    const started = await office(['start', '--port', String(await unusedLoopbackPort())]);
    const writes: WorldWrite[] = [];
    page.on('request', (request) => {
      if (new URL(request.url()).pathname === '/api/v1/local/world' && request.method() === 'PUT')
        writes.push(request.postDataJSON());
    });
    const saveResponse = () =>
      page.waitForResponse(
        (response) =>
          new URL(response.url()).pathname === '/api/v1/local/world' &&
          response.request().method() === 'PUT'
      );
    try {
      await page.setViewportSize({ width: 1440, height: 1000 });
      await page.goto(started.url);
      await openKeyboardSelection(page);
      if (initial.layout.map.version !== 8) throw new Error('Expected current platform modules');
      const modules = initial.layout.map.modules;
      const lobbyId = initial.layout.map.primaryLobbyId;
      await page.getByRole('combobox', { name: 'Area', exact: true }).selectOption(lobbyId);
      const outside: WorldDocument = {
        ...initial.layout,
        map: {
          ...initial.layout.map,
          modules: modules.map((module) =>
            module.area.id === lobbyId
              ? { ...module, area: { ...module.area, name: 'External Lobby' } }
              : module
          ),
        },
      };
      writeFileSync(file, JSON.stringify(outside));
      await office(['layout', 'apply', '--file', file, '--if-revision', '1']);
      const winner: WorldSnapshot = await office(['layout', 'show']);
      await expect(page.getByRole('button', { name: 'Refresh office' })).toBeEnabled();
      const failed = saveResponse();
      await page.getByRole('textbox', { name: 'Area name', exact: true }).fill('My unsaved Lobby');
      await expect(page.getByRole('textbox', { name: 'Area name', exact: true })).toHaveValue(
        'My unsaved Lobby'
      );
      const response = await failed;
      expect(response.status()).toBe(409);
      expect(await response.json()).toMatchObject({ error: 'WORLD_REVISION_CONFLICT' });
      await expect(page.getByRole('alert')).toContainText(
        'Nothing was written. Your draft is kept.'
      );
      await expect(page.getByRole('textbox', { name: 'Area name', exact: true })).toHaveValue(
        'My unsaved Lobby'
      );
      expect(writes).toHaveLength(1);
      expect(writes[0]).toMatchObject({
        expectedRevision: 1,
        layout: { objects: initial.layout.objects, map: { version: 8, primaryLobbyId: lobbyId } },
      });
      const draftMap = writes[0]!.layout.map;
      if (draftMap.version !== 8) throw new Error('Browser lost platform source');
      expect(draftMap.modules).toEqual(
        modules
          .map((module) =>
            module.area.id === lobbyId
              ? { ...module, area: { ...module.area, name: 'My unsaved Lobby' } }
              : module
          )
          .sort((a, b) => a.area.id.localeCompare(b.area.id))
      );
      expect(await office(['layout', 'show'])).toEqual(winner);
      expectStoredWorld(sandbox.database, winner);
      await page.screenshot({ path: info.outputPath('world-conflict-draft-preserved.png') });
      // Real error feedback must share available space, not cover editable controls.
      for (const viewport of [
        { width: 1440, height: 1000 },
        { width: 740, height: 600 },
        { width: 320, height: 568 },
      ]) {
        await page.setViewportSize(viewport);
        const controls = page.getByRole('region', { name: 'Office controls' });
        const camera = page.getByRole('group', { name: 'Map view' });
        const inspector = page.getByRole('complementary', { name: 'Layout tools' });
        const saveBar = page.getByRole('region', { name: 'Layout changes' });
        const headerBox = (await controls.boundingBox())!;
        const cameraBox = (await camera.boundingBox())!;
        const inspectorBox = (await inspector.boundingBox())!;
        const saveBox = (await saveBar.boundingBox())!;
        expect(
          cameraBox.x >= headerBox.x + headerBox.width ||
            cameraBox.y >= headerBox.y + headerBox.height
        ).toBe(true);
        expect(saveBox.y).toBeGreaterThanOrEqual(
          Math.max(headerBox.y + headerBox.height, cameraBox.y + cameraBox.height)
        );
        expect(inspectorBox.y).toBeGreaterThanOrEqual(saveBox.y + saveBox.height);
        expect(inspectorBox.y + inspectorBox.height).toBeLessThanOrEqual(viewport.height);
        expect(await page.locator('.office-canvas').boundingBox()).toMatchObject({
          x: 0,
          y: 0,
          ...viewport,
        });
        await page.getByRole('button', { name: 'Fit office', exact: true }).click();
        await openKeyboardSelection(page);
        await page.getByRole('combobox', { name: 'Object', exact: true }).click();
        await page.keyboard.press('Escape');
        await page.getByRole('combobox', { name: 'Area', exact: true }).selectOption(lobbyId);
        await page.getByRole('textbox', { name: 'Area name', exact: true }).click();
        await expect(page.getByRole('textbox', { name: 'Area name', exact: true })).toHaveValue(
          'My unsaved Lobby'
        );
        // Focusing a descendant must not scroll an overflowing HUD ancestor
        // sideways and expose the page underneath the full-screen canvas.
        expect(await page.locator('.office-canvas').boundingBox()).toMatchObject({
          x: 0,
          y: 0,
          ...viewport,
        });
        expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(
          viewport.width
        );
        await page.screenshot({ path: info.outputPath(`world-conflict-${viewport.width}.png`) });
      }
      const repeated = saveResponse();
      await page.getByRole('button', { name: 'Retry changes', exact: true }).click();
      expect((await repeated).status()).toBe(409);
      expect(writes).toHaveLength(2);
      expect(writes[1]).toEqual(writes[0]);
      expect(await office(['layout', 'show'])).toEqual(winner);
      expectStoredWorld(sandbox.database, winner);
      await page
        .getByRole('button', { name: 'Reload saved layout (discard local changes)' })
        .click();
      await expect(page.getByRole('textbox', { name: 'Area name', exact: true })).toHaveValue(
        'External Lobby'
      );
      const successful = saveResponse();
      await page.getByRole('textbox', { name: 'Area name', exact: true }).fill('Reconciled Lobby');
      expect((await successful).status()).toBe(200);
      expect(writes[2]!.expectedRevision).toBe(winner.revision);
      const saved: WorldSnapshot = await office(['layout', 'show']);
      expect(saved.revision).toBe(winner.revision + 1);
      expect(mapGeometry(saved.layout.map).areas.find((area) => area.id === lobbyId)!.name).toBe(
        'Reconciled Lobby'
      );
      expect(saved.layout.objects).toEqual(initial.layout.objects);
      expectStoredWorld(sandbox.database, saved);
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
