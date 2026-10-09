import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { describe, expect, it } from 'vite-plus/test';

const root = path.resolve(import.meta.dirname, '../../..');
const action = readFileSync(
  path.join(root, '.github/actions/setup-docker-mirror/action.yml'),
  'utf8'
);
const script = action
  .split('      run: |\n')[1]
  .split('\n')
  .map((line) => line.slice(8))
  .join('\n');

describe('hosted Docker mirror setup', () => {
  function run(mode: string, contents = '{"live-restore":true}') {
    const directory = mkdtempSync(path.join(tmpdir(), 'tmt-docker-mirror-'));
    try {
      const config = path.join(directory, 'daemon.json');
      writeFileSync(config, contents);
      if (mode === 'symlink') {
        rmSync(config);
        symlinkSync(path.join(directory, 'foreign.json'), config);
        writeFileSync(path.join(directory, 'foreign.json'), contents);
      }
      const mocks = `
record() { printf '%s\\n' "$*" >> "$MIRROR_TEST_ROOT/calls"; }
sudo() { record sudo "$@"; "$@"; }
install() {
  record install "$@"
  if [ "$MIRROR_TEST_MODE" = install-failure ]; then
    printf '{"partial":true}' > "$DAEMON_CONFIG"
    return 1
  fi
  command install "$@"
}
dockerd() { record dockerd "$@"; [ "$MIRROR_TEST_MODE" != validate-failure ]; }
systemctl() {
  record systemctl "$@"
  if [ "$1" = restart ] && [ "$MIRROR_TEST_MODE" = restart-failure ]; then return 1; fi
  if [ "$1" = restart ] || { [ "$MIRROR_TEST_MODE" != restart ] && [ "$MIRROR_TEST_MODE" != restart-failure ]; }; then
    python3 -c 'import json,os,pathlib; pathlib.Path(os.environ["MIRROR_TEST_ROOT"],"state").write_text(json.dumps(json.loads(pathlib.Path(os.environ["DAEMON_CONFIG"]).read_text()).get("registry-mirrors",[])))'
  fi
}
docker() {
  record docker "$@"
  if [ -f "$MIRROR_TEST_ROOT/state" ]; then cat "$MIRROR_TEST_ROOT/state"; else printf '[]\\n'; fi
}
`;
      const result = spawnSync('/bin/bash', ['-c', mocks + script], {
        encoding: 'utf8',
        env: {
          ...process.env,
          PATH: '/usr/bin:/bin:/opt/homebrew/bin',
          DAEMON_CONFIG: config,
          MIRROR_TEST_MODE: mode,
          MIRROR_TEST_ROOT: directory,
        },
      });
      return {
        status: result.status,
        output: result.stdout + result.stderr,
        config: readFileSync(config, 'utf8'),
        calls: (() => {
          try {
            return readFileSync(path.join(directory, 'calls'), 'utf8');
          } catch {
            return '';
          }
        })(),
      };
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  }

  it('preserves daemon fields and reloads before considering a restart', () => {
    const result = run('reload');
    expect(result.status).toBe(0);
    expect(JSON.parse(result.config)).toEqual({
      'live-restore': true,
      'registry-mirrors': ['https://mirror.gcr.io'],
      debug: true,
    });
    expect(result.calls).toContain('systemctl kill --kill-who=main --signal=HUP docker');
    expect(result.calls).not.toContain('systemctl restart');
    expect(result.output).toContain('SIGHUP reload took effect.');
  });

  it('restarts only when the mirror was not observed after reload', () => {
    const result = run('restart');
    expect(result.status).toBe(0);
    expect(result.calls.indexOf('systemctl kill')).toBeLessThan(
      result.calls.indexOf('systemctl restart')
    );
    expect(result.output).toContain('reload not observed');
  });

  it.each(['validate-failure', 'install-failure', 'restart-failure', 'symlink'])(
    'fails open and preserves original config for %s',
    (mode) => {
      const result = run(mode);
      expect(result.status).toBe(0);
      expect(result.config).toBe('{"live-restore":true}');
      expect(result.output).toContain('::warning::Docker mirror setup failed');
      expect(result.output).toContain('(best effort).');
    }
  );

  it('fails open on malformed configuration before installing or signalling', () => {
    const result = run('reload', 'not json');
    expect(result.status).toBe(0);
    expect(result.config).toBe('not json');
    expect(result.calls).toBe('');
  });

  it('wires both daemon consumers and both independent container builders', () => {
    const ci = readFileSync(path.join(root, '.github/workflows/ci.yml'), 'utf8');
    const seed = readFileSync(
      path.join(root, '.github/workflows/e2e-dependency-cache.yml'),
      'utf8'
    );
    expect(ci.match(/uses: \.\/\.github\/actions\/setup-docker-mirror/g)).toHaveLength(2);
    expect(seed).toContain('uses: ./.github/actions/setup-docker-mirror');
    for (const source of [ci, seed]) {
      expect(source).toContain('[registry."docker.io"]');
      expect(source).toContain('mirrors = ["mirror.gcr.io"]');
      expect(source).toContain('continue-on-error: true');
      expect(source).toContain('Fall back to the original Buildx setup');
    }
    expect(ci.indexOf('Configure the hosted Docker mirror')).toBeLessThan(
      ci.indexOf('Set up Buildx only')
    );
    expect(ci.lastIndexOf('Configure the hosted Docker mirror')).toBeLessThan(
      ci.indexOf('Build the verification-only Alpine environment')
    );
  });
});
