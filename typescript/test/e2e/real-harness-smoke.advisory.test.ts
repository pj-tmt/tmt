import { execFileSync } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import {
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  symlinkSync,
  writeFileSync,
} from 'node:fs';
import path from 'node:path';
import { beforeAll, expect, it } from 'vite-plus/test';
import { MOCK_MODEL_TOKEN, startMockModelServer } from '../support/mock-model-server.mjs';
import { withE2EFixture, type E2EFixture } from './harness.js';
import { waitForFileContent } from './wait-for-file.js';

const reportDirectory = '/tmp/real-harness-results';
const versions: Record<string, string> = {};
const pins = JSON.parse(readFileSync(new URL('./real-harness-pins.json', import.meta.url), 'utf8'));
type Provider = 'claude' | 'codex';
interface ListedIdentity {
  name: string;
  runningDriver?: string;
  resume?: { driver: string; session: string };
  session?: { unbound: boolean | null };
}

function quote(value: string): string {
  return `'${value.replaceAll("'", "'\\''")}'`;
}

// Keep the public launch pattern local, as in run-harness.e2e.test.ts.
function submit(
  fixture: E2EFixture,
  pane: string,
  args: string[],
  statusFile: string,
  env: Record<string, string>,
  outputFile?: string
): void {
  const command = [
    'env',
    ...Object.entries(env).map(([key, value]) => `${key}=${value}`),
    fixture.executables.cli.executable,
    ...fixture.executables.cli.args,
    ...args,
  ]
    .map(quote)
    .join(' ');
  fixture.tmux([
    'send-keys',
    '-t',
    pane,
    '-l',
    `${command}${outputFile ? ` > ${quote(outputFile)} 2>&1` : ''}; printf '%s' "$?" > ${quote(statusFile)}`,
  ]);
  fixture.tmux(['send-keys', '-t', pane, 'Enter']);
}

beforeAll(() => {
  if (!existsSync('/.dockerenv') || process.env.TMT_REAL_HARNESS !== '1')
    throw new Error('HARNESS_CONTAINER_REQUIRED: hosted advisory container only');
  if (readdirSync('/sys/class/net').some((name) => name !== 'lo'))
    throw new Error('HARNESS_NETWORK_ISOLATION: only loopback may exist');
  if (process.env.TMUX || process.env.TMUX_PANE)
    throw new Error('HARNESS_HOST_CONTEXT: unset inherited tmux context');
  mkdirSync(reportDirectory, { recursive: true, mode: 0o700 });
  mkdirSync(process.env.HOME!, { recursive: true, mode: 0o700 });
  // Both exact versions must pass before either provider receives a scripted turn.
  for (const provider of ['claude', 'codex'] as const) {
    versions[provider] = execFileSync(`/opt/real-harness/bin/${provider}`, ['--version'], {
      encoding: 'utf8',
      timeout: 15_000,
      maxBuffer: 4096,
    }).trim();
    expect(versions[provider], `HARNESS_VERSION: ${provider}`).toBe(pins.versions[provider]);
  }
  writeFileSync(path.join(reportDirectory, 'versions.json'), JSON.stringify(versions, null, 2));
});

it.each(['claude', 'codex'] as const)(
  '%s completes a diagnostic text turn with matching public hook session evidence',
  async (provider: Provider) => {
    const started = performance.now();
    const marker = `fixture-reply-${randomUUID()}`;
    const protocol = provider === 'claude' ? 'messages' : 'responses';
    const model = provider === 'claude' ? 'claude-sonnet-4-6' : 'gpt-5.4';
    const report: Record<string, unknown> = {
      provider,
      version: versions[provider],
      checkDiagnosticOnly: true,
      sessionCondition: 'not yet proven',
    };
    const mock = await startMockModelServer({
      turns: [{ protocol, model, text: marker, inputTokens: 12, outputTokens: 8 }],
      sessionSource: provider === 'claude' ? 'claude-code-2.1.285' : 'codex-0.160.0',
      // Observed in hosted run 40072755557; no other side route is assumed.
      sideRequests:
        provider === 'claude'
          ? [{ method: 'HEAD', path: '/api/hello', maxCalls: 1, response: {} }]
          : [],
    });
    try {
      await withE2EFixture(async (fixture) => {
        const home = path.join(fixture.root, 'provider-home');
        const claudeHome = path.join(home, '.claude');
        const codexHome = path.join(home, '.codex');
        const stableBin = fixture.createWorkspace('stable-bin');
        symlinkSync(fixture.executables.cli.executable, path.join(stableBin, 'tmt'));
        for (const directory of [home, claudeHome, codexHome])
          mkdirSync(directory, { recursive: true, mode: 0o700 });
        const env = {
          PATH: `${stableBin}:${fixture.wrapperDir}:${process.env.PATH}`,
          HOME: home,
          CLAUDE_CONFIG_DIR: claudeHome,
          CODEX_HOME: codexHome,
          TMT_AGENT: '1',
          BROWSER: '/bin/true',
          ANTHROPIC_BASE_URL: mock.baseUrl,
          ANTHROPIC_AUTH_TOKEN: MOCK_MODEL_TOKEN,
          HARNESS_MODEL_TOKEN: MOCK_MODEL_TOKEN,
          CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: '1',
          DISABLE_AUTOUPDATER: '1',
          DISABLE_TELEMETRY: '1',
          DISABLE_ERROR_REPORTING: '1',
        };
        writeFileSync(
          // The provider relocates this file too when CLAUDE_CONFIG_DIR is set.
          path.join(claudeHome, '.claude.json'),
          JSON.stringify({
            hasCompletedOnboarding: true,
            projects: { [fixture.workspace]: { hasTrustDialogAccepted: true } },
          })
        );
        writeFileSync(
          path.join(codexHome, 'config.toml'),
          [
            `model = ${JSON.stringify(model)}`,
            'model_provider = "fixture"',
            'approval_policy = "never"',
            'sandbox_mode = "danger-full-access"',
            'check_for_update_on_startup = false',
            'cli_auth_credentials_store = "ephemeral"',
            '[analytics]',
            'enabled = false',
            '[feedback]',
            'enabled = false',
            '[model_providers.fixture]',
            'name = "isolated fixture"',
            `base_url = ${JSON.stringify(`${mock.baseUrl}/v1`)}`,
            'wire_api = "responses"',
            'env_key = "HARNESS_MODEL_TOKEN"',
            'requires_openai_auth = false',
            'supports_websockets = false',
            `[projects.${JSON.stringify(fixture.workspace)}]`,
            'trust_level = "trusted"',
            '',
          ].join('\n')
        );
        expect(fixture.socketPath).toContain(fixture.socketRoot);
        expect(path.basename(fixture.socketPath)).not.toBe('default');
        const pane = fixture.createShellPane(`real-${provider}`).pane;
        const setupStatus = path.join(fixture.root, 'setup.status');
        const setupOutput = path.join(fixture.root, 'setup.json');
        submit(
          fixture,
          pane,
          ['setup', provider, '--yes', '--json'],
          setupStatus,
          env,
          setupOutput
        );
        const setupCode = await waitForFileContent(setupStatus, { timeoutMs: 15_000 });
        const setupText = readFileSync(setupOutput, 'utf8').slice(0, 8192);
        report.publicSetup = { code: setupCode, output: setupText };
        expect(setupCode, setupText).toBe('0');
        const name = `real-${provider}`;
        const status = path.join(fixture.root, 'provider.status');
        const args =
          provider === 'claude'
            ? ['--permission-mode', 'bypassPermissions', '--model', model]
            : ['--dangerously-bypass-approvals-and-sandbox', '--dangerously-bypass-hook-trust'];
        submit(fixture, pane, ['run', name, provider, ...args], status, env);
        try {
          let identity: ListedIdentity | undefined;
          await expect
            .poll(
              async () => {
                const result = await fixture.runJsonCli<{ identities: ListedIdentity[] }>(['ls']);
                identity = result.json?.identities.find((row) => row.name === name);
                return identity?.resume?.session;
              },
              { timeout: 45_000, interval: 500 }
            )
            .toBeTruthy();
          // The response marker is never typed: terminal input echo cannot satisfy this check.
          fixture.tmux([
            'send-keys',
            '-t',
            pane,
            '-l',
            'Reply briefly with plain text. Do not use tools.',
          ]);
          fixture.tmux(['send-keys', '-t', pane, 'Enter']);
          await expect
            .poll(
              async () => {
                const snapshot = mock.snapshot();
                if (snapshot.failure)
                  throw new Error(`${snapshot.failure.code}: ${snapshot.failure.message}`);
                const result = await fixture.runJsonCli<{ output: string }>(['check', name, '0']);
                return result.code === 0 && result.json?.output.includes(marker);
              },
              { timeout: 60_000, interval: 500 }
            )
            .toBe(true);
          const result = await fixture.runJsonCli<{ identities: ListedIdentity[] }>(['ls']);
          identity = result.json?.identities.find((row) => row.name === name);
          report.publicIdentity = identity;
          report.diagnosticMarkerObserved = true;
          const observed = mock.snapshot().census.find((row) => row.kind === 'turn')?.sessionId;
          expect(
            observed,
            'HARNESS_SESSION_NOT_PROVEN: pinned wire location not observed'
          ).toBeTruthy();
          expect(identity?.resume?.driver).toBe(provider);
          expect(identity?.resume?.session).toBe(observed);
          expect(identity?.session?.unbound).toBe(false);
          report.sessionCondition = 'matched public session to mock UUID';
          expect(mock.snapshot().completedTurns).toBe(1);
          fixture.tmux(['send-keys', '-t', pane, 'C-c', 'C-c']);
          await waitForFileContent(status, { timeoutMs: 10_000 });
        } catch (error) {
          // Public JSON and our own exit marker only: no hook files or transcripts.
          const listed = await fixture.runJsonCli<{ identities: ListedIdentity[] }>(['ls']);
          report.publicStartup = {
            code: listed.code,
            identity: listed.json?.identities.find((row) => row.name === name),
            stderr: listed.stderr.slice(0, 2048),
            exitStatus: existsSync(status) ? readFileSync(status, 'utf8').slice(0, 16) : null,
          };
          const diagnostic = await fixture.runJsonCli<{ output: string }>(['check', name, '0']);
          report.publicCheckDiagnostic = {
            code: diagnostic.code,
            output: diagnostic.json?.output?.slice(0, 8192),
            stderr: diagnostic.stderr.slice(0, 2048),
          };
          if (provider === 'codex' && diagnostic.json?.output?.includes('warning')) {
            // The previous hosted prompt explicitly offered F2 to view this warning.
            // Read it; do not approve, trust, or change provider settings through UI.
            fixture.tmux(['send-keys', '-t', pane, 'F2']);
            let warning = diagnostic;
            await expect
              .poll(
                async () => {
                  warning = await fixture.runJsonCli<{ output: string }>(['check', name, '0']);
                  return warning.code === 0 && warning.json?.output !== diagnostic.json?.output;
                },
                { timeout: 3000, interval: 100 }
              )
              .toBe(true)
              .catch(() => {});
            report.publicWarningDiagnostic = {
              code: warning.code,
              changed: warning.json?.output !== diagnostic.json?.output,
              output: warning.json?.output?.slice(0, 8192),
              stderr: warning.stderr.slice(0, 2048),
            };
          }
          throw error;
        }
      });
      report.qualified = true;
    } catch (error) {
      report.qualified = false;
      report.failure = error instanceof Error ? error.message.slice(0, 2048) : 'Unknown failure';
      throw error;
    } finally {
      await mock.close();
      report.mock = mock.snapshot();
      report.seconds = Number(((performance.now() - started) / 1000).toFixed(3));
      writeFileSync(
        path.join(reportDirectory, `${provider}.json`),
        JSON.stringify(report, null, 2)
      );
    }
  }
);
