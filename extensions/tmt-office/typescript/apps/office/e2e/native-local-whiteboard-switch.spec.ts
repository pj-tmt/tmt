import { writeFileSync } from 'node:fs';
import path from 'node:path';
import Database from '../../../../../../typescript/test/support/sqlite-oracle.js';
import { expect, test } from '@playwright/test';
import { runCli, withSandbox } from '../../../../../../typescript/test/support/cli-process.js';
import { officeWorldFixture } from '../../../../../../typescript/test/support/office-world.js';
import { WHITEBOARD_EXTENSION as whiteboard } from '../src/extensions/bundled-extensions.js';
import { installNativeOffice, unusedLoopbackPort } from './native-office-fixture.js';
import { openOfficeObjects } from './office-navigation.js';

test('switching whiteboards preserves drafts and resolves an uncertain save without duplicate writes', async ({
  page,
}) => {
  test.setTimeout(120_000);
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
    const secondId = '40000000-0000-4000-8000-000000000002';
    const file = path.join(sandbox.root, 'two-boards.json');
    writeFileSync(
      file,
      JSON.stringify({
        ...officeWorldFixture().layout,
        objects: ['lobby', secondId].map((documentId, i) => ({
          id: `30000000-0000-4000-8000-00000000000${i + 1}`,
          kind: 'decoration',
          surface: { type: 'floor' },
          placement: { ...whiteboard.appearance, x: i * 16 + 2, y: 2, rotation: 0 },
          extension: {
            definition: whiteboard.id,
            binding: { kind: 'whiteboard', documentId },
          },
        })),
      })
    );
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
    const stored = () => {
      const db = new Database(sandbox.database, { readonly: true });
      try {
        return {
          boards: db
            .prepare('SELECT document_id, revision FROM office_whiteboards ORDER BY document_id')
            .all(),
          operations: db.prepare('SELECT count(*) FROM office_whiteboard_operations').pluck().get(),
        };
      } finally {
        db.close();
      }
    };
    const started = await office(['start', '--port', String(await unusedLoopbackPort())]);
    try {
      await page.setViewportSize({ width: 1440, height: 1000 });
      await page.goto(started.url);
      await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
      await openOfficeObjects(page);
      const entries = page.getByRole('button', { name: 'Open whiteboard', exact: true });
      const board = page.getByRole('dialog', { name: 'Whiteboard', exact: true });
      const close = () =>
        board.getByRole('button', { name: 'Close whiteboard', exact: true }).click();
      await entries.nth(0).click();
      await expect(board.getByText('New board · not saved')).toBeVisible();
      await board.getByRole('button', { name: 'Text', exact: true }).click();
      await board.getByLabel('Whiteboard drawing surface').click({ position: { x: 80, y: 80 } });
      await board.getByRole('textbox', { name: 'Text', exact: true }).fill('Keep this drawing');
      await board.getByRole('button', { name: 'Apply text', exact: true }).click();
      await close();
      await entries.nth(1).click();
      await expect(
        board.getByRole('region', { name: 'Switch whiteboard confirmation' })
      ).toBeVisible();
      await board.getByRole('button', { name: 'Keep this whiteboard', exact: true }).click();
      await expect(board.getByRole('textbox', { name: 'Text', exact: true })).toHaveValue(
        'Keep this drawing'
      );
      expect(stored()).toEqual({ boards: [], operations: 0 });

      // The native service commits, but the browser loses the response. Switching
      // must retain the original operation until its exact retry is confirmed.
      await page.route('**/api/v1/local/whiteboards/lobby', async (route) => {
        if (route.request().method() !== 'PUT') return route.continue();
        const response = await route.fetch();
        expect(response.ok()).toBe(true);
        await route.abort('failed');
      });
      await board.getByRole('button', { name: 'Save', exact: true }).click();
      await expect(board.getByText('Draft kept · save unconfirmed')).toBeVisible();
      await close();
      await entries.nth(1).click();
      await expect(
        board.getByRole('button', { name: 'Discard draft and switch', exact: true })
      ).toBeDisabled();
      expect(stored()).toEqual({ boards: [{ document_id: 'lobby', revision: 1 }], operations: 1 });
      await board.getByRole('button', { name: 'Keep this whiteboard', exact: true }).click();
      await page.unroute('**/api/v1/local/whiteboards/lobby');
      await board.getByRole('button', { name: 'Retry save', exact: true }).click();
      await expect(board.getByText('Saved · revision 1')).toBeVisible();
      expect(stored()).toEqual({ boards: [{ document_id: 'lobby', revision: 1 }], operations: 1 });

      // Snapshot composition is part of this resource session, not disposable HUD state.
      await board.getByRole('button', { name: 'Review snapshot', exact: true }).click();
      await board
        .getByRole('textbox', { name: 'Annotation', exact: true })
        .fill('Review this part');
      await close();
      await entries.nth(1).click();
      await expect(
        board.getByRole('region', { name: 'Switch whiteboard confirmation' })
      ).toBeVisible();
      await board.getByRole('button', { name: 'Keep this whiteboard', exact: true }).click();
      await expect(board.getByRole('textbox', { name: 'Annotation', exact: true })).toHaveValue(
        'Review this part'
      );
      const captureIntents: unknown[] = [];
      await page.route('**/api/v1/local/whiteboards/lobby/snapshots', async (route) => {
        captureIntents.push(route.request().postDataJSON());
        const response = await route.fetch();
        expect(response.ok()).toBe(true);
        if (captureIntents.length === 1) await route.abort('failed');
        else await route.fulfill({ response });
      });
      await board.getByRole('button', { name: 'Create snapshot', exact: true }).click();
      await expect(
        board.getByRole('button', { name: 'Retry snapshot', exact: true })
      ).toBeEnabled();
      await board.getByRole('button', { name: 'Start new preview', exact: true }).click();
      await expect(
        board.getByRole('region', { name: 'Discard unconfirmed snapshot confirmation' })
      ).toBeVisible();
      await board.getByRole('button', { name: 'Keep snapshot retry', exact: true }).click();
      await close();
      await entries.nth(1).click();
      await expect(
        board.getByRole('button', { name: 'Discard draft and switch', exact: true })
      ).toBeDisabled();
      await board.getByRole('button', { name: 'Keep this whiteboard', exact: true }).click();
      await board.getByRole('button', { name: 'Retry snapshot', exact: true }).click();
      await expect(
        board.getByRole('heading', { name: 'Snapshot ready', exact: true })
      ).toBeVisible();
      expect(captureIntents).toHaveLength(2);
      expect(captureIntents[1]).toEqual(captureIntents[0]);
      const db = new Database(sandbox.database, { readonly: true });
      try {
        expect(db.prepare('SELECT count(*) FROM office_whiteboard_snapshots').pluck().get()).toBe(
          1
        );
      } finally {
        db.close();
      }
      await page.unroute('**/api/v1/local/whiteboards/lobby/snapshots');
      await board.getByRole('button', { name: 'Back to drawing', exact: true }).click();

      // Unapplied inspector text is also a draft, even when the scene is saved.
      await board.getByRole('textbox', { name: 'Text', exact: true }).fill('Unapplied changes');
      await close();
      await entries.nth(1).click();
      await expect(
        board.getByRole('region', { name: 'Switch whiteboard confirmation' })
      ).toBeVisible();
      await board.getByRole('button', { name: 'Discard draft and switch', exact: true }).click();
      await expect(board.getByText('New board · not saved')).toBeVisible();
      await expect(board.getByText('0 elements · click to select')).toBeVisible();
      await close();
      await entries.nth(0).click();
      await expect(board.getByText('Saved · revision 1')).toBeVisible();
      await board.getByRole('button', { name: '1. text — Keep this drawing', exact: true }).click();
      await expect(board.getByRole('textbox', { name: 'Text', exact: true })).toHaveValue(
        'Keep this drawing'
      );
      expect(stored()).toEqual({ boards: [{ document_id: 'lobby', revision: 1 }], operations: 1 });
      await close();
      const direct = new URL(started.url);
      direct.pathname = '/local/whiteboards/lobby';
      await page.goto(direct.href);
      await expect(page.getByText('Saved · revision 1')).toBeVisible();
      await page.getByRole('button', { name: '1. text — Keep this drawing', exact: true }).click();
      await page.getByRole('textbox', { name: 'Text', exact: true }).fill('Direct route draft');
      await page.getByRole('link', { name: '← Back to office', exact: true }).click();
      await expect(
        page.getByRole('region', { name: 'Leave whiteboard confirmation' })
      ).toBeVisible();
      await page.getByRole('button', { name: 'Stay on this whiteboard', exact: true }).click();
      await expect(page.getByRole('textbox', { name: 'Text', exact: true })).toHaveValue(
        'Direct route draft'
      );
      await page.getByRole('button', { name: 'Apply text', exact: true }).click();
      await page.route('**/api/v1/local/whiteboards/lobby', async (route) => {
        if (route.request().method() !== 'PUT') return route.continue();
        const response = await route.fetch();
        expect(response.ok()).toBe(true);
        await route.abort('failed');
      });
      await page.getByRole('button', { name: 'Save', exact: true }).click();
      await expect(page.getByText('Draft kept · save unconfirmed')).toBeVisible();
      await page.getByRole('link', { name: '← Back to office', exact: true }).click();
      await expect(
        page.getByRole('button', { name: 'Discard draft and leave', exact: true })
      ).toBeDisabled();
      await page.getByRole('button', { name: 'Stay on this whiteboard', exact: true }).click();
      await page.unroute('**/api/v1/local/whiteboards/lobby');
      await page.getByRole('button', { name: 'Retry save', exact: true }).click();
      await expect(page.getByText('Saved · revision 2')).toBeVisible();
      await page.getByRole('textbox', { name: 'Text', exact: true }).fill('Explicitly discarded');
      await page.getByRole('link', { name: '← Back to office', exact: true }).click();
      await page.getByRole('button', { name: 'Discard draft and leave', exact: true }).click();
      await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
      expect(stored()).toEqual({ boards: [{ document_id: 'lobby', revision: 2 }], operations: 2 });
    } finally {
      await page.goto('about:blank');
      await office(['stop']);
      expect((await office(['status'])).service.running).toBe(false);
    }
  });
});
