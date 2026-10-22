import type { Reporter, TestCase, TestResult, TestStep } from '@playwright/test/reporter';
import { loadSoakDictionary, policy } from './soak-literals.js';
import { projectSoakFailure } from './soak-diagnostics.js';

/** Closed Playwright API labels only. User/test-authored step titles are never evidence. */
export function apiSpan(
  step: Pick<TestStep, 'category' | 'title' | 'startTime' | 'duration'>,
  originMs: number,
  observedMs: number,
) {
  if (step.category !== 'pw:api' || !Object.hasOwn(policy.api, step.title)) return;
  const name = policy.api[step.title as keyof typeof policy.api];
  const startMs = step.startTime.getTime() - originMs;
  const durationMs = step.duration < 0 ? observedMs - startMs : step.duration;
  return { name, startMs, durationMs };
}

/** Runs before JSON reporter, replacing private metadata with a closed API-span projection. */
export default class SoakReporter implements Reporter {
  private readonly steps = new Map<TestResult, TestStep[]>();
  onStepBegin(_test: TestCase, result: TestResult, step: TestStep) {
    if (step.category !== 'pw:api' || !Object.hasOwn(policy.api, step.title)) return;
    const steps = this.steps.get(result) ?? [];
    steps.push(step);
    if (steps.length > 512) steps.shift();
    this.steps.set(result, steps);
  }
  onTestEnd(test: TestCase, result: TestResult) {
    const steps = this.steps.get(result) ?? [];
    this.steps.delete(result);
    if (
      test.title !== policy.scenario ||
      !/[/\\]connection-soak\.spec\.ts$/.test(test.location.file) ||
      !['failed', 'timedOut', 'interrupted'].includes(result.status)
    )
      return;
    const dictionary = loadSoakDictionary();
    for (const attachment of result.attachments) {
      if (
        attachment.name !== policy.attachment ||
        attachment.contentType !== 'application/json' ||
        !attachment.body ||
        attachment.body.length > policy.tailBytes
      )
        continue;
      try {
        const input = JSON.parse(attachment.body.toString('utf8'));
        const origin = Number.isSafeInteger(input.originMs)
          ? input.originMs
          : result.startTime.getTime();
        const observedMs = input.observedMs;
        const spans = steps.flatMap((step) => {
          const span = apiSpan(step, origin, observedMs);
          return span ? [span] : [];
        });
        const projected = projectSoakFailure(
          {
            ...input,
            spans,
            truncated: {
              ...input.truncated,
              spans: input.truncated?.spans === true || steps.length >= 512,
            },
          },
          dictionary,
        );
        attachment.body = projected ? Buffer.from(JSON.stringify(projected)) : undefined;
        delete attachment.path;
      } catch {
        attachment.body = undefined;
        delete attachment.path;
      }
    }
  }
}
