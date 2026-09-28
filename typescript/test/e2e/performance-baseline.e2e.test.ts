import fs from 'node:fs';
import os from 'node:os';
import type { CliExecutables } from '../support/cli-executable.mjs';
import { assertBenchmarkHelp } from '../support/performance-contract.mjs';
import { performance } from 'node:perf_hooks';
import { describe, expect, it } from 'vitest';
import { durableState } from './identity-state-oracle.js';
import { withE2EFixture, type CliResult, type E2EFixture, type MockEvent } from './harness.js';
import { installTmuxTrace, type TmuxTrace } from './tmux-trace.js';

const ENABLED = process.env.TMT_PERFORMANCE_BASELINE === '1';
const MOCK_REPLY_DELAY_MS = process.env.TMT_PERFORMANCE_SLOW_REPLY === '1' ? 1_500 : 0;
const SAMPLE_COUNT = 7;
const WARM_SAMPLE_COUNT = SAMPLE_COUNT - 1;
const LARGE_SESSION_UNBOUND_PANES = 200;
const KNOWN_TMUX_TRACE_COMMANDS = new Set([
  'capture-pane',
  'display-message',
  'list-panes',
  'paste-buffer',
  'send-keys',
  'set-buffer',
  'set-option',
  'show-options',
]);

interface Identity {
  id: string;
  name: string;
  canonicalName: string;
}

interface IdentityResult {
  identity: Identity;
  created: boolean;
}

interface WhoamiResult {
  bound: boolean;
  pane: string;
}

interface CheckResult {
  target: string;
  pane: string;
  lines: number;
  output: string;
}

interface TalkResult {
  status: string;
  response: string;
  bodyBytes: number;
  requestId: string;
  pane: string;
  submittedAtMs: number;
}

interface ResultOutput {
  status: string;
  response: string;
  bodyBytes: number;
  requestId: string;
  submittedAtMs: number;
}

interface ConfigShowResult {
  resolved: {
    defaults: {
      timeout: number;
      pollInterval: number;
      pasteEnterDelayMs: number;
    };
  };
}

interface Observation {
  wallMs: number;
  subprocesses: number;
  commandCounts: Record<string, number>;
}

interface BenchmarkSeries {
  sampleCount: number;
  warmSampleCount: number;
  firstObservation: Observation;
  warmFreshProcess: Observation[];
  medianMs: number;
  warmMedianMs: number | null;
}

interface PerformanceBaselineReport {
  executables: CliExecutables;
  schemaVersion: 1;
  benchmark: 'tmt-performance-baseline';
  generatedAt: string;
  sampleCount: number;
  warmSampleCount: number;
  semantics: {
    firstObservation: string;
    warmFreshProcess: string;
    osCache: string;
  };
  timingDefaults: {
    timeoutSeconds: number;
    pollIntervalSeconds: number;
    pasteEnterDelayMs: number;
    benchmarkTalkTimeoutSeconds: number;
    injectedMockReplyDelayMs: number;
  };
  environment: {
    node: string;
    tmux: string;
    platform: NodeJS.Platform;
    kernel: string;
    arch: string;
  };
  resourceUsage: {
    cpu: { status: 'unavailable'; reason: string };
    peakRss: { status: 'unavailable'; reason: string };
  };
  scenarios: {
    help: BenchmarkSeries;
    identityCreate: BenchmarkSeries;
    identityShow: BenchmarkSeries;
    whoamiSmall: BenchmarkSeries;
    whoamiLarge: BenchmarkSeries;
    checkSmall: BenchmarkSeries;
    checkLarge: BenchmarkSeries;
    talkNoPreamble: BenchmarkSeries;
    result: BenchmarkSeries;
  };
}

interface MeasurementValue<T> {
  value: T;
  observation: Observation;
}

function json<T>(result: CliResult<T>): T {
  expect(result.code, result.stderr || result.stdout).toBe(0);
  expect(result.stderr).toBe('');
  expect(result.stdout.trim()).not.toBe('');
  expect(result.json).toBeDefined();
  return result.json as T;
}

function median(values: readonly number[]): number {
  expect(values.length).toBeGreaterThan(0);
  const sorted = [...values].sort((left, right) => left - right);
  const middle = Math.floor(sorted.length / 2);
  if (sorted.length % 2 === 1) return sorted[middle] ?? 0;
  return ((sorted[middle - 1] ?? 0) + (sorted[middle] ?? 0)) / 2;
}

function commandCounts(commands: readonly string[]): Record<string, number> {
  return commands.reduce<Record<string, number>>((counts, command) => {
    counts[command] = (counts[command] ?? 0) + 1;
    return counts;
  }, {});
}

function observation(wallMs: number, trace: TmuxTrace): Observation {
  const commands = trace.commands();
  return {
    wallMs: Number(wallMs.toFixed(3)),
    subprocesses: trace.invocations().length,
    commandCounts: commandCounts(commands),
  };
}

async function measure<T>(
  trace: TmuxTrace,
  action: () => Promise<T>
): Promise<MeasurementValue<T>> {
  trace.clear();
  const startedAt = performance.now();
  const value = await action();
  return { value, observation: observation(performance.now() - startedAt, trace) };
}

async function measureSeries<T>(
  trace: TmuxTrace,
  action: (sample: number) => Promise<T>,
  validate: (value: T, sample: number) => Promise<void> | void = () => {},
  sampleCount = SAMPLE_COUNT
): Promise<{ series: BenchmarkSeries; values: T[] }> {
  const first = await measure(trace, () => action(0));
  await validate(first.value, 0);
  const warm: MeasurementValue<T>[] = [];
  for (let sample = 1; sample < sampleCount; sample += 1) {
    const item = await measure(trace, () => action(sample));
    await validate(item.value, sample);
    warm.push(item);
  }
  const observations = [first.observation, ...warm.map(({ observation: item }) => item)];
  return {
    series: {
      sampleCount: observations.length,
      warmSampleCount: warm.length,
      firstObservation: first.observation,
      warmFreshProcess: warm.map(({ observation: item }) => item),
      medianMs: median(observations.map((item) => item.wallMs)),
      warmMedianMs:
        warm.length > 0 ? median(warm.map(({ observation: item }) => item.wallMs)) : null,
    },
    values: [first.value, ...warm.map(({ value }) => value)],
  };
}

function createUnboundSleepingPanes(fixture: E2EFixture): string[] {
  const panes: string[] = [];
  for (let index = 0; index < LARGE_SESSION_UNBOUND_PANES; index += 1) {
    const pane = fixture
      .tmux([
        'new-window',
        '-d',
        '-P',
        '-F',
        '#{pane_id}',
        '-t',
        'e2e',
        '-n',
        `baseline-idle-${String(index).padStart(3, '0')}`,
        '-c',
        fixture.workspace,
        'sleep 300',
      ])
      .trim();
    expect(pane).toMatch(/^%\d+$/);
    panes.push(pane);
  }
  return panes;
}

function processIsRunning(pid: number): boolean {
  if (!Number.isInteger(pid) || pid <= 0) return false;
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

function assertKnownTraceCommands(series: BenchmarkSeries): void {
  const observations = [series.firstObservation, ...series.warmFreshProcess];
  for (const item of observations) {
    expect(
      Object.keys(item.commandCounts).filter((command) => !KNOWN_TMUX_TRACE_COMMANDS.has(command))
    ).toEqual([]);
  }
}

function assertDurableSubmission(
  fixture: E2EFixture,
  result: TalkResult,
  message: string,
  pid: number
): Promise<MockEvent> {
  return fixture.waitForEvent(
    (event) =>
      event.event === 'submitted' &&
      event.requestId === result.requestId &&
      event.pid === pid &&
      event.body === `mock-agent response: ${message}`
  );
}

const baselineSuite = ENABLED ? describe.sequential : describe.skip;

baselineSuite('TMT performance baseline', () => {
  it(
    'measures fresh CLI processes against observable output, state, and tmux fan-out',
    { timeout: 120_000 },
    async () => {
      const cleanup = { root: '', socketRoot: '', serverPid: 0 };
      let scenarioFailed = false;
      let scenarioError: unknown;
      try {
        await withE2EFixture(
          async (fixture) => {
            cleanup.root = fixture.root;
            cleanup.socketRoot = fixture.socketRoot;
            cleanup.serverPid = fixture.serverPid;
            const trace = installTmuxTrace(fixture);
            const tmuxVersion = fixture.tmux(['-V']).trim();

            const help = await measureSeries(
              trace,
              () => fixture.runCli(['--help'], { withoutTmux: true }),
              (result) => {
                expect(result.code).toBe(0);
                expect(result.stderr).toBe('');
                assertBenchmarkHelp(result.stdout);
              }
            );

            const identityCreate = await measureSeries(
              trace,
              () =>
                fixture.runJsonCli<IdentityResult>(['identity', 'create', 'PerformanceBaseline'], {
                  withoutTmux: true,
                }),
              (result) => {
                const output = json(result);
                expect(output).toMatchObject({
                  created: true,
                  identity: {
                    name: 'PerformanceBaseline',
                    canonicalName: 'performancebaseline',
                    id: expect.stringMatching(/^[0-9a-f-]{36}$/),
                  },
                });
                expect(durableState(fixture).identities).toHaveLength(1);
              },
              1
            );
            const createdIdentity = json(identityCreate.values[0]).identity;

            const configResult = await fixture.runJsonCli<ConfigShowResult>(['config', 'show'], {
              withoutTmux: true,
            });
            const timingDefaults = json(configResult).resolved.defaults;

            const identityShow = await measureSeries(
              trace,
              () =>
                fixture.runJsonCli<IdentityResult>(['identity', 'show', 'performancebaseline'], {
                  withoutTmux: true,
                }),
              (result) => {
                const output = json(result);
                expect(output).toMatchObject({ identity: createdIdentity });
              }
            );
            expect(durableState(fixture).identities).toHaveLength(1);

            const whoamiWarmup = await fixture.runJsonCli<WhoamiResult>(['whoami']);
            expect(json(whoamiWarmup)).toEqual({
              bound: false,
              pane: fixture.pane,
              interfaceKind: 'container',
              sessionState: 'unknown',
            });

            const whoamiSmall = await measureSeries(
              trace,
              () => fixture.runJsonCli<WhoamiResult>(['whoami']),
              (result) => {
                expect(json(result)).toEqual({
                  bound: false,
                  pane: fixture.pane,
                  interfaceKind: 'container',
                  sessionState: 'unknown',
                });
              }
            );

            const checkMarker = 'performance-baseline-check-marker';
            const markerTalk = await fixture.runJsonCli<TalkResult>([
              'talk',
              fixture.pane,
              checkMarker,
              '--no-preamble',
              '--timeout',
              '8',
            ]);
            const markerOutput = json(markerTalk);
            expect(markerOutput.response).toBe(`mock-agent response: ${checkMarker}`);
            await fixture.waitForEvent(
              (event) => event.event === 'summary' && event.requestId === markerOutput.requestId
            );

            const checkSmall = await measureSeries(
              trace,
              () => fixture.runJsonCli<CheckResult>(['check', fixture.pane, '--lines=20']),
              (result) => {
                const output = json(result);
                expect(output).toMatchObject({
                  target: fixture.pane,
                  pane: fixture.pane,
                  lines: 20,
                });
                expect(output.output).toContain(`mock-agent summary: ${checkMarker}`);
              }
            );

            const unboundPanes = createUnboundSleepingPanes(fixture);
            expect(unboundPanes).toHaveLength(LARGE_SESSION_UNBOUND_PANES);
            expect(unboundPanes.every((pane) => fixture.paneMetadata(pane) === '')).toBe(true);

            const whoamiLarge = await measureSeries(
              trace,
              () => fixture.runJsonCli<WhoamiResult>(['whoami']),
              (result) => {
                expect(json(result)).toEqual({
                  bound: false,
                  pane: fixture.pane,
                  interfaceKind: 'container',
                  sessionState: 'unknown',
                });
              }
            );
            const checkLarge = await measureSeries(
              trace,
              () => fixture.runJsonCli<CheckResult>(['check', fixture.pane, '--lines=20']),
              (result) => {
                const output = json(result);
                expect(output).toMatchObject({
                  target: fixture.pane,
                  pane: fixture.pane,
                  lines: 20,
                });
                expect(output.output).toContain(`mock-agent summary: ${checkMarker}`);
              }
            );

            expect(whoamiLarge.series.firstObservation.subprocesses).toBe(
              whoamiSmall.series.firstObservation.subprocesses
            );
            expect(whoamiLarge.series.firstObservation.commandCounts).toEqual(
              whoamiSmall.series.firstObservation.commandCounts
            );
            expect(checkLarge.series.firstObservation.subprocesses).toBe(
              checkSmall.series.firstObservation.subprocesses
            );
            expect(checkLarge.series.firstObservation.commandCounts).toEqual(
              checkSmall.series.firstObservation.commandCounts
            );
            expect(whoamiLarge.series.warmFreshProcess.map((item) => item.subprocesses)).toEqual(
              whoamiSmall.series.warmFreshProcess.map((item) => item.subprocesses)
            );
            expect(checkLarge.series.warmFreshProcess.map((item) => item.subprocesses)).toEqual(
              checkSmall.series.warmFreshProcess.map((item) => item.subprocesses)
            );

            const talkRequestIds: string[] = [];
            const talkNoPreamble = await measureSeries(
              trace,
              (sample) => {
                const message = `performance-baseline-${sample}`;
                return fixture.runJsonCli<TalkResult>([
                  'talk',
                  fixture.pane,
                  message,
                  '--no-preamble',
                  '--timeout',
                  '8',
                ]);
              },
              async (result, sample) => {
                const message = `performance-baseline-${sample}`;
                const output = json(result);
                expect(output).toMatchObject({
                  status: 'completed',
                  pane: fixture.pane,
                  response: `mock-agent response: ${message}`,
                  bodyBytes: Buffer.byteLength(`mock-agent response: ${message}`),
                  requestId: expect.stringMatching(/^req_[0-9a-f-]+$/),
                  submittedAtMs: expect.any(Number),
                });
                await assertDurableSubmission(fixture, output, message, fixture.panePid);
                talkRequestIds.push(output.requestId);
              }
            );

            let resultSample = 0;
            const result = await measureSeries(
              trace,
              () =>
                fixture.runJsonCli<ResultOutput>(['result', talkRequestIds[resultSample++] ?? ''], {
                  withoutTmux: true,
                }),
              (command, sample) => {
                const requestId = talkRequestIds[sample];
                const response = `mock-agent response: performance-baseline-${sample}`;
                const talkOutput = json(talkNoPreamble.values[sample]);
                const output = json(command);
                expect(requestId).toMatch(/^req_[0-9a-f-]+$/);
                expect(output).toEqual({
                  status: 'completed',
                  requestId,
                  response,
                  bodyBytes: Buffer.byteLength(response),
                  submittedAtMs: talkOutput.submittedAtMs,
                });
              }
            );
            expect(fs.existsSync(fixture.forbiddenTmuxLogPath)).toBe(false);
            for (const series of [
              help.series,
              identityCreate.series,
              identityShow.series,
              whoamiSmall.series,
              whoamiLarge.series,
              checkSmall.series,
              checkLarge.series,
              talkNoPreamble.series,
              result.series,
            ]) {
              assertKnownTraceCommands(series);
            }

            const report: PerformanceBaselineReport = {
              executables: fixture.executables,
              schemaVersion: 1,
              benchmark: 'tmt-performance-baseline',
              generatedAt: new Date().toISOString(),
              sampleCount: SAMPLE_COUNT,
              warmSampleCount: WARM_SAMPLE_COUNT,
              semantics: {
                firstObservation:
                  'The first observation runs after fixture setup and is reported separately; it is not an OS-cache-cold measurement.',
                warmFreshProcess:
                  'Each warm sample launches a new CLI process after the first observation.',
                osCache:
                  'Not controlled or claimed cold; filesystem and process caches may be warm.',
              },
              timingDefaults: {
                timeoutSeconds: timingDefaults.timeout,
                pollIntervalSeconds: timingDefaults.pollInterval,
                pasteEnterDelayMs: timingDefaults.pasteEnterDelayMs,
                benchmarkTalkTimeoutSeconds: 8,
                injectedMockReplyDelayMs: MOCK_REPLY_DELAY_MS,
              },
              environment: {
                node: process.version,
                tmux: tmuxVersion,
                platform: process.platform,
                kernel: os.release(),
                arch: process.arch,
              },
              resourceUsage: {
                cpu: {
                  status: 'unavailable',
                  reason:
                    'The existing E2E harness exposes child PIDs but no child CPU accounting; parent Vitest usage is not a child metric.',
                },
                peakRss: {
                  status: 'unavailable',
                  reason:
                    'The existing E2E harness does not sample child peak RSS; parent Vitest RSS is not a child metric.',
                },
              },
              scenarios: {
                help: help.series,
                identityCreate: identityCreate.series,
                identityShow: identityShow.series,
                whoamiSmall: whoamiSmall.series,
                whoamiLarge: whoamiLarge.series,
                checkSmall: checkSmall.series,
                checkLarge: checkLarge.series,
                talkNoPreamble: talkNoPreamble.series,
                result: result.series,
              },
            };

            process.stdout.write(`TMT_PERFORMANCE_BASELINE ${JSON.stringify(report)}\n`);
          },
          { replyDelayMs: MOCK_REPLY_DELAY_MS }
        );
      } catch (error) {
        scenarioFailed = true;
        scenarioError = error;
      }
      if (cleanup.root !== '') {
        expect(fs.existsSync(cleanup.root)).toBe(false);
        expect(fs.existsSync(cleanup.socketRoot)).toBe(false);
        expect(processIsRunning(cleanup.serverPid)).toBe(false);
      }
      if (scenarioFailed) throw scenarioError;
    }
  );
});
