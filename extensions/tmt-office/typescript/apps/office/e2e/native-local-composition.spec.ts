import { writeFileSync } from 'node:fs';
import path from 'node:path';
import { expect, test } from '@playwright/test';
import { withSandbox } from '../../../../../../typescript/test/support/cli-process.js';
import { legacyLobbyObjects } from '../../../../../../typescript/test/support/office-world.js';
import { withE2EFixture } from '../../../../../../typescript/test/e2e/harness.js';
import { installNativeOffice, unusedLoopbackPort } from './native-office-fixture.js';
import {
  openAgentDetails,
  openKeyboardSelection,
  openOfficeDirectory,
} from './office-navigation.js';
import { savedWorld } from './native-world-state.js';
import { workshopStarter } from '../src/blocks/workshop-starter.js';
import type { WorldDocument, WorldObject } from '../src/world-map/world-contract.js';
import type { WorldSnapshot } from '../src/world-map/world-port.js';
import type { MeetingRoom } from '../src/local/room-contract.js';

/** Combined visual/interaction acceptance, not a replacement for lifecycle/failure scenarios. */
test('a furnished native office keeps personal, Contractor and meeting experiences in one world', async ({
  page,
}, info) => {
  await withSandbox(async (sandbox) => {
    const prefix = await installNativeOffice(sandbox);
    await withE2EFixture(
      async (fixture) => {
        async function cli<T>(args: string[]): Promise<T> {
          const result = await fixture.runJsonCli<T>(args);
          expect(result.code, result.stdout + result.stderr).toBe(0);
          expect(result.json).toBeDefined();
          return result.json!;
        }
        const office = <T>(args: string[]) => cli<T>(['office', '--prefix', prefix, ...args]);
        const people = [];
        for (const [name, saved] of [
          ['Alice', true],
          ['Bob', true],
          ['Pip', false],
        ] as const) {
          const pane = await fixture.createMockPane(name.toLowerCase());
          const identity = await cli<{ id: string }>([
            'add',
            pane.pane,
            name,
            ...(saved ? ['-s'] : []),
          ]);
          people.push({ ...identity, name });
        }
        await cli(['identity', 'create', 'Offline']);
        for (const [index, person] of people.slice(0, 2).entries()) {
          const file = path.join(sandbox.root, `${person.name}-profile.json`);
          writeFileSync(
            file,
            JSON.stringify({
              displayLabel: `${person.name}'s studio`,
              description: 'A quiet place to build and review.',
              appearance: {
                hairStyle: index === 0 ? 'bald' : 'short',
                hairColor: 'silver',
                skinTone: index === 0 ? 'light' : 'warm',
                shirtColor: index === 0 ? 'blue' : 'green',
                shirtMark: index === 0 ? 'AI' : 'OPS',
              },
            })
          );
          await office([
            'profile',
            'apply',
            '--local',
            '--identity',
            person.name,
            '--file',
            file,
            '--if-revision',
            '0',
          ]);
        }
        await cli([
          'identity',
          'status',
          'set',
          'Reviewing the layout',
          '--mood',
          'focused',
          '--identity',
          'Alice',
        ]);
        await cli([
          'identity',
          'status',
          'set',
          'Testing controls',
          '--mood',
          'curious',
          '--identity',
          'Pip',
        ]);
        const { room } = await cli<{ room: MeetingRoom }>(['room', 'create', 'Design review']);
        for (const person of people)
          await cli(['room', 'join', room.id, '--identity', person.name]);
        const initial = await office<WorldSnapshot>(['layout', 'show']);
        const lobby = initial.layout.map.primaryLobbyId;
        const studios = [
          {
            id: '70000000-0000-4000-8000-000000000001',
            name: 'Alice studio',
            binding: { type: 'personal' as const, identityId: people[0]!.id },
          },
          {
            id: '70000000-0000-4000-8000-000000000002',
            name: 'Bob studio',
            binding: { type: 'personal' as const, identityId: people[1]!.id },
          },
        ];
        const meeting = {
          id: '70000000-0000-4000-8000-000000000003',
          name: 'Design review',
          binding: { type: 'meeting' as const, roomId: room.id },
        };
        const objects: WorldObject[] = legacyLobbyObjects().map((object) => ({
          ...object,
          placement: {
            ...object.placement,
            x: object.placement.x + 36,
            y: object.placement.y + 40,
          },
        }));
        // Existing recipes seed furnishings only; assertions use native storage and browser behavior.
        for (const [index, style] of (['study', 'library'] as const).entries())
          for (const placement of workshopStarter(style))
            objects.push({
              id: `30000000-0000-4000-8000-${String(objects.length + 1).padStart(12, '0')}`,
              kind: 'decoration',
              surface: { type: 'floor' },
              extension: null,
              placement: { ...placement, x: placement.x + index * 36 + 2, y: placement.y + 2 },
            });
        const areas = [...studios, meeting];
        const layout: WorldDocument = {
          version: 1,
          objects,
          map: {
            version: 1,
            primaryLobbyId: lobby,
            areas: [{ id: lobby, name: 'Lobby', binding: { type: 'lobby' } }, ...areas],
            floor: [
              ...Array.from({ length: 36 }, (_, y) =>
                areas.map((area, index) => ({
                  y,
                  start: index * 36,
                  end: (index + 1) * 36,
                  areaId: area.id,
                }))
              ).flat(),
              ...Array.from({ length: 4 }, (_, y) => ({
                y: y + 36,
                start: 0,
                end: 108,
                areaId: null,
              })),
              ...Array.from({ length: 36 }, (_, y) => ({
                y: y + 40,
                start: 0,
                end: 108,
                areaId: lobby,
              })),
            ],
            doors: [14, 15, 16, 17, 50, 51, 52, 53, 86, 87, 88, 89].flatMap((x) =>
              [36, 40].map((y) => ({
                x,
                y,
                axis: 'horizontal' as const,
              }))
            ),
          },
        };
        const file = path.join(sandbox.root, 'furnished-world.json');
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
        const before = savedWorld(sandbox.database);
        expect(JSON.parse(before.layout)).toEqual(layout);
        const started = await office<{ url: string }>([
          'start',
          '--port',
          String(await unusedLoopbackPort()),
        ]);
        const errors: string[] = [];
        page.on('pageerror', (error) => errors.push(error.message));
        try {
          await page.setViewportSize({ width: 1440, height: 1000 });
          await page.goto(started.url);
          await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
          const furnishedWrite = page.waitForResponse((response) => {
            if (
              new URL(response.url()).pathname !== '/api/v1/local/world' ||
              response.request().method() !== 'PUT'
            )
              return false;
            const input = response.request().postDataJSON() as { layout?: WorldDocument };
            const added = input.layout?.objects.slice(objects.length) ?? [];
            return (
              added.length === 18 &&
              areas.every((target) =>
                added.some((object) => object.placement.customization?.text === target.name)
              )
            );
          });
          const canvas = page.locator('.office-canvas canvas');
          const canvasNode = await canvas.elementHandle();
          const fullViewport = await canvas.boundingBox();
          expect(fullViewport).toMatchObject({ x: 0, y: 0, width: 1440, height: 1000 });
          await openKeyboardSelection(page);
          const area = page.getByRole('combobox', { name: 'Area', exact: true });
          await area.selectOption(meeting.id);
          await page.getByRole('button', { name: 'Add meeting set', exact: true }).click();
          for (const target of areas) {
            await area.selectOption(target.id);
            for (const name of ['Observatory window', 'Brass wall lamp', 'Crew sign']) {
              await page.getByRole('button', { name: 'Clear selection', exact: true }).click();
              await page.getByRole('button', { name, exact: true }).click();
            }
            await page.getByLabel('Display text', { exact: true }).fill(target.name);
            await page.getByRole('button', { name: 'Apply appearance', exact: true }).click();
          }
          const acknowledged = await furnishedWrite;
          expect(acknowledged.status()).toBe(200);
          await page.screenshot({ path: info.outputPath('furnished-autosaved-desktop.png') });
          await expect(
            page.getByRole('region', { name: 'Layout changes' }).getByRole('status')
          ).toHaveText('All changes applied');
          const stored = savedWorld(sandbox.database);
          const completed: WorldDocument = JSON.parse(stored.layout);
          const receipt = (await acknowledged.json()) as WorldSnapshot;
          expect(stored.worldId).toBe(before.worldId);
          expect(stored.revision).toBeGreaterThan(before.revision);
          expect(stored.revision).toBe(receipt.revision);
          expect(completed).toEqual(receipt.layout);
          expect(completed.map).toEqual(layout.map);
          expect(completed.objects.slice(0, objects.length)).toEqual(objects);
          const added = completed.objects.slice(objects.length);
          expect(added).toHaveLength(18);
          expect(added.slice(0, 3).map((object) => object.extension?.binding)).toEqual([
            { kind: 'whiteboard', documentId: room.id },
            { kind: 'office-board', roomId: room.id },
            { kind: 'office-broadcast' },
          ]);
          expect(added.slice(9).map((object) => object.kind)).toEqual([
            'window',
            'wallLight',
            'decoration',
            'window',
            'wallLight',
            'decoration',
            'window',
            'wallLight',
            'decoration',
          ]);
          expect(
            added
              .slice(9)
              .map((object) => object.placement.customization?.text)
              .filter(Boolean)
          ).toEqual(areas.map((target) => target.name));
          expect(completed.objects.filter((object) => object.kind === 'window')).toHaveLength(3);
          expect(completed.objects.filter((object) => object.kind === 'wallLight')).toHaveLength(3);
          await page.getByRole('button', { name: 'Fit office', exact: true }).click();
          await page.screenshot({ path: info.outputPath('furnished-overview-desktop.png') });

          await openAgentDetails(page, 'Alice');
          await expect(page.getByRole('region', { name: 'Self-reported status' })).toContainText(
            'Reviewing the layout'
          );
          await expect(page.getByText('Saved identity · Online', { exact: true })).toBeVisible();
          await page.screenshot({ path: info.outputPath('furnished-agent-info.png') });
          const messageAlice = page.getByRole('button', { name: 'Message Alice', exact: true });
          await messageAlice.evaluate((element) => element.scrollIntoView({ block: 'center' }));
          await expect(messageAlice).toBeInViewport();
          await messageAlice.click();
          const chat = page.getByRole('dialog', { name: 'Agent conversation', exact: true });
          await expect(
            chat.getByText('Start a conversation with Alice.', { exact: true })
          ).toBeVisible();
          await chat
            .getByRole('textbox', { name: 'Message', exact: true })
            .fill('Review this office layout?');
          await page.screenshot({ path: info.outputPath('furnished-agent-chat.png') });
          // Composition remains a draft; delivery/recovery is covered by native conversation scenarios.
          await chat.getByRole('textbox', { name: 'Message', exact: true }).fill('');
          await chat.getByRole('button', { name: 'Close agent conversation' }).click();
          await openAgentDetails(page, 'Pip');
          await expect(page.getByText('Contractor · Online', { exact: true })).toBeVisible();
          await page.getByRole('button', { name: 'Close agent conversation' }).click();
          await openOfficeDirectory(page);
          await page.getByRole('button', { name: /^Design review · meeting/ }).click();
          await expect(
            page.getByRole('heading', { name: 'Members · 3', exact: true })
          ).toBeVisible();
          const actions = page.getByRole('navigation', { name: 'Office objects' });
          await expect(
            actions.getByRole('button', { name: 'Open whiteboard', exact: true })
          ).toBeVisible();
          await page.screenshot({ path: info.outputPath('furnished-meeting-desktop.png') });
          await page.getByRole('button', { name: 'Message room', exact: true }).click();
          const message = page.getByRole('dialog', { name: 'Message room', exact: true });
          await expect(
            message.getByRole('heading', { name: 'Message Design review', exact: true })
          ).toBeVisible();
          await message.getByRole('button', { name: 'Use this roster', exact: true }).click();
          await message
            .getByRole('textbox', { name: 'Message', exact: true })
            .fill('Review the office layout together.');
          await message.getByRole('button', { name: 'Review request', exact: true }).click();
          const recipients = message.getByRole('list', { name: 'Confirmed recipients' });
          await expect(recipients.getByRole('listitem')).toHaveCount(3);
          expect((await recipients.getByRole('listitem').allTextContents()).sort()).toEqual([
            'Alice',
            'Bob',
            'Pip',
          ]);
          await page.screenshot({ path: info.outputPath('furnished-room-message.png') });
          await message.getByRole('button', { name: 'Close message room', exact: true }).click();
          await page.getByRole('button', { name: 'Clear selection', exact: true }).click();
          expect(await canvas.boundingBox()).toEqual(fullViewport);
          expect(await canvasNode!.evaluate((element) => element.isConnected)).toBe(true);

          for (const viewport of [
            { width: 740, height: 600 },
            { width: 320, height: 568 },
            { width: 390, height: 844 },
          ]) {
            await page.setViewportSize(viewport);
            const header = (await page
              .getByRole('region', { name: 'Office controls' })
              .boundingBox())!;
            const camera = (await page.getByRole('group', { name: 'Map view' }).boundingBox())!;
            expect(
              camera.x >= header.x + header.width || camera.y >= header.y + header.height
            ).toBe(true);
            expect(camera.x + camera.width).toBeLessThanOrEqual(viewport.width);
            expect(camera.y + camera.height).toBeLessThanOrEqual(viewport.height);
            await page.getByRole('button', { name: 'Zoom in', exact: true }).click();
            await page.getByRole('button', { name: 'Zoom out', exact: true }).click();
            await openOfficeDirectory(page);
            const directory = page.getByRole('complementary', { name: 'Office directory' });
            const directorySearch = directory.getByRole('searchbox', { name: 'Search directory' });
            await directorySearch.fill('');
            const bounds = (await directory.boundingBox())!;
            await page.screenshot({
              path: info.outputPath(`furnished-directory-${viewport.width}.png`),
            });
            expect(bounds.y).toBeGreaterThanOrEqual(
              Math.max(header.y + header.height, camera.y + camera.height)
            );
            expect(bounds.x).toBeGreaterThanOrEqual(0);
            expect(bounds.x + bounds.width).toBeLessThanOrEqual(viewport.width);
            expect(bounds.y + bounds.height).toBeLessThanOrEqual(viewport.height);
            for (const control of await page.locator('.world-hud-topline button:visible').all())
              await expect(control).toBeInViewport();
            await expect(directorySearch).toBeInViewport();
            await directorySearch.fill('Pip');
            await expect(directory.getByRole('button', { name: /^Pip · Online/ })).toBeInViewport();
            await page.getByRole('button', { name: 'Zoom in', exact: true }).click();
            await page.getByRole('button', { name: 'Zoom out', exact: true }).click();
            await page.keyboard.press('Escape');
          }
          await page.getByRole('button', { name: 'Fit office', exact: true }).click();
          await page.screenshot({ path: info.outputPath('furnished-overview-narrow.png') });
          await openAgentDetails(page, 'Pip');
          await expect(page.getByText('Contractor · Online', { exact: true })).toBeInViewport();
          expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(390);
          expect(await canvas.boundingBox()).toMatchObject({ x: 0, y: 0, width: 390, height: 844 });
          await page.screenshot({ path: info.outputPath('furnished-contractor-narrow.png') });
          expect(savedWorld(sandbox.database)).toEqual(stored);

          // Retiring a resident changes the active roster, not their retained office.
          await page.getByRole('button', { name: 'Close agent conversation' }).click();
          await cli(['rm', 'Alice', '--force']);
          await page.getByRole('button', { name: 'Refresh office', exact: true }).click();
          for (const restarted of [false, true]) {
            if (restarted) {
              await page.goto('about:blank');
              await office(['stop']);
              const next = await office<{ url: string }>([
                'start',
                '--port',
                String(await unusedLoopbackPort()),
              ]);
              await page.goto(next.url);
            }
            await expect(page.getByRole('button', { name: 'Office menu' })).toBeVisible();
            await openOfficeDirectory(page);
            const directory = page.getByRole('complementary', { name: 'Office directory' });
            await directory.getByRole('searchbox', { name: 'Search directory' }).fill('');
            await expect(directory.getByRole('button', { name: /^Alice ·/ })).toHaveCount(0);
            await directory.getByRole('button', { name: 'Alice studio · personal' }).click();
            await expect(page.getByRole('heading', { name: 'Residents · 0' })).toBeVisible();
            expect(savedWorld(sandbox.database)).toEqual(stored);
            const remaining = (await cli<{ room: MeetingRoom }>(['room', 'show', room.id])).room;
            expect(remaining.memberIds.sort()).toEqual(
              people
                .slice(1)
                .map((p) => p.id)
                .sort()
            );
            await page.getByRole('button', { name: 'Clear selection', exact: true }).click();
          }
          expect(errors).toEqual([]);
        } finally {
          try {
            if (!page.isClosed()) await page.goto('about:blank');
          } finally {
            await office(['stop']);
            expect(
              (await office<{ service: { running: boolean } }>(['status'])).service.running
            ).toBe(false);
          }
        }
      },
      { globalDir: sandbox.globalDir }
    );
  });
});
