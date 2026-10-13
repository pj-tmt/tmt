import { spawnSync } from 'node:child_process';
import { copyFileSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vite-plus/test';

const root = fileURLToPath(new URL('../../../', import.meta.url));

describe('Firebase configuration isolation', () => {
  it('ignores local settings and secrets but keeps shared setup trackable', () => {
    const ignored = [
      '.firebaserc',
      'fixture/services/browser/.firebaserc',
      'fixture/services/browser/.env.local',
      'fixture/apps/browser/.env.production',
      'fixture/services/browser/.firebase/cache.json',
      'fixture/services/browser/.firebase-local/export/data.json',
      'fixture/services/browser/.secrets/arbitrary-name.json',
      'fixture/services/browser/project-firebase-adminsdk-fixture.json',
      'fixture/services/browser/service-account.json',
      'fixture/services/browser/application_default_credentials.json',
      'fixture/services/browser/private.pem',
      'fixture/services/browser/private.key',
    ];
    const shared = [
      'fixture/services/browser/.firebaserc.example',
      'fixture/services/browser/.env.example',
      'fixture/services/browser/firebase.json',
      'fixture/services/browser/firestore.rules',
      'fixture/services/browser/compose.yaml',
      'fixture/services/browser/Dockerfile',
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
});
