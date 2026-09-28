import { readFileSync, writeFileSync } from 'node:fs';
import { get } from 'node:http';
import path from 'node:path';
import Database from '../../../../../../typescript/test/support/sqlite-oracle.js';
import { expect, test } from '@playwright/test';
import type { BrowserContext } from '@playwright/test';
import { runCli, withSandbox } from '../../../../../../typescript/test/support/cli-process.js';
import { installNativeOffice, unusedLoopbackPort } from './native-office-fixture.js';
import { openAgentDetails } from './office-navigation.js';

function durableProfile(databasePath: string, identityId: string) {
  const database = new Database(databasePath, { readonly: true });
  try {
    const profile = database
      .prepare(
        'SELECT identity_id, revision, profile, updated_at_ms FROM office_local_profiles WHERE identity_id = ?'
      )
      .get(identityId) as
      | { identity_id: string; revision: number; profile: string; updated_at_ms: number }
      | undefined;
    return {
      profile: profile && { ...profile, profile: JSON.parse(profile.profile) },
      role: database
        .prepare('SELECT identity_id, content, updated_at FROM role_profiles WHERE identity_id = ?')
        .get(identityId),
      world: database
        .prepare(
          'SELECT id, layout_revision, layout_json, layout_updated_at_ms FROM office_local_worlds'
        )
        .get(),
    };
  } finally {
    database.close();
  }
}

async function verifyEmbeddedAssets(sessionUrl: string): Promise<void> {
  const index = await fetch(sessionUrl);
  expect(index.ok).toBe(true);
  const html = await index.text();
  const source = html.match(/<script[^>]+src="([^"]+\.js)"/)?.[1];
  if (!source) throw new Error('Embedded Office index has no module asset.');
  const result = await new Promise<{ declared: number; received: number; complete: boolean }>(
    (resolve, reject) => {
      const request = get(new URL(source, sessionUrl), (response) => {
        let received = 0;
        response.on('data', (chunk: Buffer) => (received += chunk.byteLength));
        response.on('end', () =>
          resolve({
            declared: Number(response.headers['content-length']),
            received,
            complete: response.complete,
          })
        );
        response.on('aborted', () =>
          resolve({
            declared: Number(response.headers['content-length']),
            received,
            complete: false,
          })
        );
      });
      request.on('error', reject);
    }
  );
  expect(result).toEqual({ declared: result.declared, received: result.declared, complete: true });
}

test('real local CLI profile reaches SQLite, browser and service restart', async ({
  browser,
  page,
}, testInfo) => {
  test.setTimeout(150_000);
  await withSandbox(async (sandbox) => {
    const prefix = await installNativeOffice(sandbox);
    // Unicode names exercise identity resolution and safe browser text rendering.
    const identityName = '\u8a2d\u8a08\u5718\u968a 🚀';
    const identity = await runCli(sandbox, ['identity', 'create', identityName, '--json']);
    expect(identity.status, identity.stdout).toBe(0);
    const identityId = JSON.parse(identity.stdout).identity.id as string;
    const role = await runCli(sandbox, [
      'role',
      'set',
      'Review architecture',
      '--identity',
      identityName,
      '--json',
    ]);
    expect(role.status, role.stdout).toBe(0);
    const displayLabel = 'Architecture '.repeat(7).slice(0, 80);
    const profileFile = path.join(sandbox.root, 'profile.json');
    writeFileSync(
      profileFile,
      JSON.stringify({
        displayLabel,
        description: '<script>plain text only</script>',
        appearance: {
          hairStyle: 'tied',
          hairColor: 'gold',
          skinTone: 'warm',
          shirtColor: 'green',
          shirtMark: '🚀🚀',
        },
      })
    );
    const office = (args: string[]) =>
      runCli(sandbox, ['office', '--prefix', prefix, ...args, '--json'], { deadlineMs: 30_000 });
    const implicit = await office(['profile', 'show', '--local']);
    expect(implicit.status).toBe(1);
    expect(implicit.stdout).toMatch(/PANE_NOT_FOUND|IDENTITY_REQUIRED/);
    const shown = await office(['profile', 'show', '--local', '--identity', identityName]);
    expect(shown.status, shown.stdout).toBe(0);
    expect(JSON.parse(shown.stdout)).toMatchObject({
      exists: false,
      revision: 0,
      identityName,
    });
    const invalidFile = path.join(sandbox.root, 'invalid-profile.json');
    writeFileSync(
      invalidFile,
      JSON.stringify({ ...JSON.parse(readFileSync(profileFile, 'utf8')), url: 'x' })
    );
    const invalid = await office([
      'profile',
      'apply',
      '--local',
      '--identity',
      identityName,
      '--file',
      invalidFile,
      '--if-revision',
      '0',
    ]);
    expect(invalid.status).toBe(1);
    expect(invalid.stdout).toContain('PROFILE_INVALID');
    expect(durableProfile(sandbox.database, identityId).profile).toBeUndefined();
    const applied = await office([
      'profile',
      'apply',
      '--local',
      '--identity',
      identityName,
      '--file',
      profileFile,
      '--if-revision',
      '0',
    ]);
    expect(applied.status, applied.stdout).toBe(0);
    const createdProfile = JSON.parse(applied.stdout);
    expect(createdProfile).toMatchObject({
      exists: true,
      revision: 1,
      changed: true,
      profile: { displayLabel },
    });
    const exactCreateRetry = await office([
      'profile',
      'apply',
      '--local',
      '--identity',
      identityName,
      '--file',
      profileFile,
      '--if-revision',
      '0',
    ]);
    expect(exactCreateRetry.status, exactCreateRetry.stdout).toBe(0);
    expect(JSON.parse(exactCreateRetry.stdout)).toMatchObject({
      revision: 1,
      changed: false,
      updatedAtMs: createdProfile.updatedAtMs,
    });
    const currentNoop = await office([
      'profile',
      'apply',
      '--local',
      '--identity',
      identityName,
      '--file',
      profileFile,
      '--if-revision',
      '1',
    ]);
    expect(currentNoop.status, currentNoop.stdout).toBe(0);
    expect(JSON.parse(currentNoop.stdout)).toMatchObject({
      revision: 1,
      changed: false,
      updatedAtMs: createdProfile.updatedAtMs,
    });
    const updatedProfile = {
      ...createdProfile.profile,
      appearance: { ...createdProfile.profile.appearance, hairColor: 'silver' },
    };
    writeFileSync(profileFile, JSON.stringify(updatedProfile));
    const updated = await office([
      'profile',
      'apply',
      '--local',
      '--identity',
      identityName,
      '--file',
      profileFile,
      '--if-revision',
      '1',
    ]);
    expect(updated.status, updated.stdout).toBe(0);
    expect(JSON.parse(updated.stdout)).toMatchObject({ revision: 2, changed: true });
    const preview = await office(['layout', 'show']);
    expect(preview.status, preview.stdout).toBe(0);
    const initialWorld = JSON.parse(preview.stdout);
    // Profile updates must never replace the independent, already-saved world.
    const layoutFile = path.join(sandbox.root, 'world.json');
    writeFileSync(layoutFile, JSON.stringify(initialWorld.layout));
    const layout = await office([
      'layout',
      'apply',
      '--file',
      layoutFile,
      '--if-revision',
      '0',
      '--legacy-basis',
      initialWorld.legacyBasis,
    ]);
    expect(layout.status, layout.stdout).toBe(0);
    const initialState = durableProfile(sandbox.database, identityId);
    expect(initialState.profile).toMatchObject({
      identity_id: identityId,
      revision: 2,
      profile: { displayLabel, appearance: { hairColor: 'silver', shirtMark: '🚀🚀' } },
      updated_at_ms: expect.any(Number),
    });
    expect(initialState.role).toMatchObject({ content: 'Review architecture' });
    expect(initialState.world).toMatchObject({
      layout_revision: 1,
      layout_json: JSON.stringify(initialWorld.layout),
    });
    const initialDiagnostics: Array<Promise<string>> = [];
    page.on('pageerror', (error) =>
      initialDiagnostics.push(Promise.resolve(`pageerror: ${error.message}`))
    );
    page.on('requestfailed', (request) =>
      initialDiagnostics.push(
        Promise.resolve(
          `requestfailed: ${request.url()} ${request.failure()?.errorText ?? 'unknown'}`
        )
      )
    );
    page.on('response', (response) => {
      if (response.url().includes('/api/v1/local/profiles'))
        initialDiagnostics.push(
          response.text().then((body) => `profiles: ${response.status()} ${body}`)
        );
      else if (!response.ok())
        initialDiagnostics.push(
          Promise.resolve(`response: ${response.status()} ${response.url()}`)
        );
    });
    // The full browser suite runs native scenarios in parallel. Give this
    // sandbox its own loopback port instead of racing for the default port.
    let started = await office(['start', '--port', String(await unusedLoopbackPort())]);
    expect(started.status, started.stdout).toBe(0);
    let restartedContext: BrowserContext | undefined;
    try {
      const initialUrl = JSON.parse(started.stdout).url;
      try {
        await verifyEmbeddedAssets(initialUrl);
      } catch (cause) {
        const status = await office(['status']);
        throw new Error(`Embedded asset failed; Office status: ${status.stdout}`, { cause });
      }
      await page.goto(initialUrl);
      await openAgentDetails(page, identityName);
      await page.getByRole('button', { name: 'Appearance', exact: true }).click();
      await expect(page.getByLabel('Description')).toBeVisible();
      try {
        await expect(page.locator('.profile-preview .avatar-name')).toHaveText(identityName);
      } catch (error) {
        await testInfo.attach('native-profile-initial-diagnostics', {
          body: Buffer.from(
            `${(await Promise.all(initialDiagnostics)).join('\n')}\n${await page.locator('body').innerText()}`
          ),
          contentType: 'text/plain',
        });
        throw error;
      }
      await expect(page.locator('.profile-preview .avatar-label')).toHaveText(displayLabel);
      await expect(page.locator('.profile-preview .avatar-mark')).toHaveText('🚀🚀');
      await expect(page.getByLabel('Description')).toHaveValue('<script>plain text only</script>');
      expect(await page.locator('script').count()).toBeGreaterThan(0);
      expect(await page.locator('.profile-editor script').count()).toBe(0);
      await expect(page.getByText('Saved identity · Offline', { exact: true })).toBeVisible();
      await expect(page.getByRole('complementary', { name: 'Office directory' })).toBeHidden();
      await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
      await page.screenshot({ path: testInfo.outputPath('native-profile-initial.png') });
      await page.getByLabel('Description').fill('Saved from browser 🚀');
      await page.getByRole('button', { name: 'Save appearance' }).click();
      await expect(page.getByText('Saved · revision 3')).toBeVisible();
      const browserState = durableProfile(sandbox.database, identityId);
      expect(browserState.profile).toMatchObject({
        revision: 3,
        profile: { description: 'Saved from browser 🚀' },
        updated_at_ms: expect.any(Number),
      });
      expect(browserState.role).toEqual(initialState.role);
      expect(browserState.world).toEqual(initialState.world);
      writeFileSync(profileFile, JSON.stringify(browserState.profile?.profile));
      const exactRetry = await office([
        'profile',
        'apply',
        '--local',
        '--identity',
        identityName,
        '--file',
        profileFile,
        '--if-revision',
        '2',
      ]);
      expect(exactRetry.status, exactRetry.stdout).toBe(0);
      expect(JSON.parse(exactRetry.stdout)).toMatchObject({ revision: 3, changed: false });
      expect(durableProfile(sandbox.database, identityId)).toEqual(browserState);
      writeFileSync(profileFile, JSON.stringify(updatedProfile));
      const stale = await office([
        'profile',
        'apply',
        '--local',
        '--identity',
        identityName,
        '--file',
        profileFile,
        '--if-revision',
        '2',
      ]);
      expect(stale.status).toBe(1);
      expect(JSON.parse(stale.stdout).error.code).toBe('OFFICE_REVISION_CONFLICT');
      expect(durableProfile(sandbox.database, identityId)).toEqual(browserState);
      await page.setViewportSize({ width: 390, height: 844 });
      await page.screenshot({
        path: testInfo.outputPath('native-profile-narrow.png'),
        fullPage: true,
      });
      // Reopen in a fresh browser context after the service restart. This also
      // discards the previous origin's connection pool and revoked token.
      await page.close();
      expect((await office(['stop'])).status).toBe(0);
      started = await office(['start', '--port', String(await unusedLoopbackPort())]);
      expect(started.status, started.stdout).toBe(0);
      restartedContext = await browser.newContext();
      const restartedPage = await restartedContext.newPage();
      const restartDiagnostics: string[] = [];
      restartedPage.on('pageerror', (error) =>
        restartDiagnostics.push(`pageerror: ${error.message}`)
      );
      restartedPage.on('requestfailed', (request) =>
        restartDiagnostics.push(
          `requestfailed: ${request.url()} ${request.failure()?.errorText ?? 'unknown'}`
        )
      );
      restartedPage.on('response', (response) => {
        if (!response.ok())
          restartDiagnostics.push(`response: ${response.status()} ${response.url()}`);
      });
      const restartedUrl = JSON.parse(started.stdout).url;
      await verifyEmbeddedAssets(restartedUrl);
      const restartedResponse = await restartedPage.goto(restartedUrl);
      expect(restartedResponse?.ok()).toBe(true);
      try {
        await expect(restartedPage.getByRole('region', { name: 'Office overview' })).toBeVisible();
        await openAgentDetails(restartedPage, identityName);
        await restartedPage.getByRole('button', { name: 'Appearance', exact: true }).click();
        await expect(restartedPage.getByLabel('Description')).toBeVisible();
      } catch (error) {
        await testInfo.attach('native-profile-restart-diagnostics', {
          body: Buffer.from(
            `${restartDiagnostics.join('\n')}\n${(await restartedPage.content()).slice(0, 4096)}`
          ),
          contentType: 'text/plain',
        });
        throw error;
      }
      await expect(restartedPage.locator('.profile-preview .avatar-mark')).toHaveText('🚀🚀');
      await expect(restartedPage.getByLabel('Description')).toHaveValue('Saved from browser 🚀');
      await restartedPage.screenshot({ path: testInfo.outputPath('native-profile-restart.png') });
      expect(
        JSON.parse(
          (await office(['profile', 'show', '--local', '--identity', identityName])).stdout
        )
      ).toMatchObject({ revision: 3 });
      expect((await office(['stop'])).status).toBe(0);
      const retired = await runCli(sandbox, ['rm', identityName, '--force', '--json']);
      expect(retired.status, retired.stdout).toBe(0);
      const retiredShow = await office(['profile', 'show', '--local', '--identity', identityName]);
      expect(retiredShow.status).toBe(3);
      expect(retiredShow.stdout).toContain('NAME_NOT_FOUND');
      expect(durableProfile(sandbox.database, identityId).profile).toEqual(browserState.profile);
      expect(durableProfile(sandbox.database, identityId).world).toEqual(initialState.world);
      const replacement = await runCli(sandbox, ['identity', 'create', identityName, '--json']);
      expect(replacement.status, replacement.stdout).toBe(0);
      const replacementId = JSON.parse(replacement.stdout).identity.id as string;
      expect(replacementId).not.toBe(identityId);
      const replacementProfile = await office([
        'profile',
        'show',
        '--local',
        '--identity',
        identityName,
      ]);
      expect(replacementProfile.status, replacementProfile.stdout).toBe(0);
      expect(JSON.parse(replacementProfile.stdout)).toMatchObject({
        identityId: replacementId,
        exists: false,
        revision: 0,
      });
      // A reused name is a new identity, not a reset of the installation world
      // or a license to erase the retired identity's retained presentation.
      expect(durableProfile(sandbox.database, replacementId).profile).toBeUndefined();
      expect(durableProfile(sandbox.database, replacementId).world).toEqual(initialState.world);
      expect(durableProfile(sandbox.database, identityId).profile).toEqual(browserState.profile);
    } finally {
      await restartedContext?.close();
      const stopped = await office(['stop']);
      expect(stopped.status, stopped.stdout).toBe(0);
    }
  });
});
