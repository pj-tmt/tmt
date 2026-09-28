import { spawnSync } from 'node:child_process';
import { copyFileSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const root = fileURLToPath(new URL('../../../', import.meta.url));

describe('Firebase configuration isolation', () => {
  it('ignores local settings and secrets but keeps shared setup trackable', () => {
    const ignored = [
      '.firebaserc',
      'extensions/tmt-office/typescript/services/office/.firebaserc',
      'extensions/tmt-office/typescript/services/office/.env.local',
      'extensions/tmt-office/typescript/apps/office/.env.production',
      'extensions/tmt-office/typescript/services/office/.firebase/cache.json',
      'extensions/tmt-office/typescript/services/office/.firebase-local/export/data.json',
      'extensions/tmt-office/typescript/services/office/.secrets/arbitrary-name.json',
      'extensions/tmt-office/typescript/services/office/project-firebase-adminsdk-fixture.json',
      'extensions/tmt-office/typescript/services/office/service-account.json',
      'extensions/tmt-office/typescript/services/office/application_default_credentials.json',
      'extensions/tmt-office/typescript/services/office/private.pem',
      'extensions/tmt-office/typescript/services/office/private.key',
    ];
    const shared = [
      'extensions/tmt-office/typescript/services/office/.firebaserc.example',
      'extensions/tmt-office/typescript/services/office/.env.example',
      'extensions/tmt-office/typescript/services/office/firebase.json',
      'extensions/tmt-office/typescript/services/office/firestore.rules',
      'extensions/tmt-office/typescript/services/office/compose.yaml',
      'extensions/tmt-office/typescript/services/office/Dockerfile',
    ];
    // Test only the versioned rules; local/global excludes could mask a missing
    // rule in the developer checkout and produce a false pass.
    const fixture = mkdtempSync(path.join(tmpdir(), 'tmt-firebase-ignore-'));
    try {
      const initialized = spawnSync('git', ['init', '--quiet', '--template=', fixture], {
        encoding: 'utf8',
        timeout: 5000,
      });
      expect(initialized.status).toBe(0);
      copyFileSync(path.join(root, '.gitignore'), path.join(fixture, '.gitignore'));
      const result = spawnSync(
        'git',
        ['-c', 'core.excludesFile=/dev/null', 'check-ignore', '--no-index', '-z', '--stdin'],
        {
          cwd: fixture,
          input: [...ignored, ...shared].join('\0') + '\0',
          encoding: 'utf8',
          timeout: 5000,
        }
      );
      expect(result.error).toBeUndefined();
      expect(result.status).toBe(0);
      expect(result.stderr).toBe('');
      expect(new Set(result.stdout.split('\0').filter(Boolean))).toEqual(new Set(ignored));
    } finally {
      rmSync(fixture, { recursive: true, force: true });
    }
  });

  it('keeps owner mapping opt-in with no real-project default in the shared template', () => {
    const template = JSON.parse(
      readFileSync(
        new URL(
          '../../../extensions/tmt-office/typescript/services/office/.firebaserc.example',
          import.meta.url
        ),
        'utf8'
      )
    );
    expect(template).toEqual({ projects: { owner: 'replace-with-your-project-id' } });
    const config = JSON.parse(
      readFileSync(
        new URL(
          '../../../extensions/tmt-office/typescript/services/office/firebase.json',
          import.meta.url
        ),
        'utf8'
      )
    );
    expect(Object.keys(config.emulators).sort()).toEqual([
      'auth',
      'firestore',
      'functions',
      'hub',
      'logging',
      'singleProjectMode',
      'ui',
    ]);
    expect(config.firestore.rules).toBe('firestore.rules');
    expect(config.emulators.ui.enabled).toBe(false);
    expect(config.emulators.singleProjectMode).toBe(true);
    expect(config.hosting).toBeUndefined();
    expect(config.functions).toEqual({
      source: 'functions',
      codebase: 'office',
      runtime: 'nodejs22',
    });
    expect(config.emulators.functions).toEqual({ host: '127.0.0.1', port: 5001 });
  });
});
