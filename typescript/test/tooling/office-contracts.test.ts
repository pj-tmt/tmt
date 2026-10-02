import { readFileSync } from 'node:fs';
import { Ajv2020 } from 'ajv/dist/2020.js';
import { describe, expect, it } from 'vite-plus/test';
import examples from '../../../extensions/tmt-office/contracts/examples.json' with { type: 'json' };
import scenarios from '../../../extensions/tmt-office/contracts/scenarios.json' with { type: 'json' };

const schema = JSON.parse(
  readFileSync(
    new URL('../../../extensions/tmt-office/contracts/v1.schema.json', import.meta.url),
    'utf8'
  )
);
const ajv = new Ajv2020({ strict: true, allErrors: true });
const validate = ajv.compile(schema);

// These tests execute the wire schema, not a fake implementation of future
// authorization or dispatch policy. Behavioral vectors remain downstream work.
describe('Office wire structure', () => {
  it.each(Object.entries(examples))(
    'accepts the %s example without rewriting it',
    (_name, input) => {
      const before = structuredClone(input);
      expect(validate(input), ajv.errorsText(validate.errors)).toBe(true);
      expect(input).toEqual(before);
    }
  );

  it('permits unknown handshake offers structurally, not as compatible sessions', () => {
    expect(
      validate({ ...examples.hello, supportedVersions: ['2.0'], requiredCapabilities: ['future'] })
    ).toBe(true);
    expect(validate({ ...examples.submit, version: '2.0' })).toBe(false);
    expect(validate({ ...examples.submit, version: '1.1' })).toBe(false);
  });

  it.each([
    { kind: 'arbitrary.execute' },
    { expiresAtMs: -1 },
    { expiresAtMs: 1.5 },
    { expiresAtMs: Number.MAX_SAFE_INTEGER + 1 },
    { expiresAtMs: '1800003600000' },
    { worldId: '../private/world' },
    { agentId: '%14' },
    { agentId: 'alice' },
    { capability: 'shell' },
    { actorUid: 'claimed-owner' },
    { receipt: 'local-proof' },
    { command: 'sh' },
    { extra: true },
  ])('rejects malformed or injected submit fields %j', (patch) => {
    expect(validate(examples.submit)).toBe(true);
    expect(validate({ ...examples.submit, ...patch })).toBe(false);
  });

  it('requires every mandatory submit field and rejects unknown nested review fields', () => {
    for (const field of [
      'kind',
      'version',
      'worldId',
      'exchangeId',
      'agentId',
      'capability',
      'text',
      'expiresAtMs',
      'review',
    ]) {
      const incomplete: Record<string, unknown> = { ...examples.submit };
      delete incomplete[field];
      expect(validate(incomplete), field).toBe(false);
    }
    expect(
      validate({ ...examples.submit, review: { ...examples.submit.review, path: '/private' } })
    ).toBe(false);
    expect(
      validate({ ...examples.submit, review: { ...examples.submit.review, headSha: 'main' } })
    ).toBe(false);
  });

  it('enforces scalar boundaries without corrupting valid astral or multiline text', () => {
    // Astral text is deliberate Unicode boundary data, not localized repo prose.
    const scalar = String.fromCodePoint(0x1f680);
    for (const text of ['line one\nline two', 'a'.repeat(4096), scalar.repeat(4096)]) {
      expect(validate({ ...examples.submit, text })).toBe(true);
      expect(Buffer.byteLength(text, 'utf8')).toBeLessThanOrEqual(16 * 1024);
    }
    for (const text of ['a'.repeat(4097), scalar.repeat(4097), '\ud800', '\udfff']) {
      expect(validate({ ...examples.submit, text })).toBe(false);
    }
    const final = (text: string) => ({ ...examples.final, result: { state: 'available', text } });
    expect(validate(final(scalar.repeat(16384)))).toBe(true);
    expect(Buffer.byteLength(scalar.repeat(16384), 'utf8')).toBe(64 * 1024);
    expect(validate(final(scalar.repeat(16385)))).toBe(false);
    expect(validate(final('\ud800'))).toBe(false);
  });

  it('preserves native empty exact text instead of inventing a withheld reason', () => {
    expect(validate({ ...examples.submit, text: '' })).toBe(true);
    expect(validate({ ...examples.final, result: { state: 'available', text: '' } })).toBe(true);
    expect(validate({ ...examples.submit, text: null })).toBe(false);
  });

  it('does not mix withheld metadata with private content or arbitrary blob references', () => {
    expect(
      validate({ ...examples.withheld, result: { state: 'withheld', reason: 'too_large' } })
    ).toBe(true);
    for (const result of [
      { state: 'withheld', reason: 'export_not_approved', text: 'private' },
      { state: 'available' },
      { state: 'available', text: 'result', url: 'https://example.invalid/private' },
      { state: 'withheld', reason: 'raw exception containing a secret' },
    ]) {
      expect(validate({ ...examples.final, result })).toBe(false);
    }
  });

  it('allows bounded native evidence but not native error documents or endpoint data', () => {
    for (const evidence of ['prepared', 'sent', 'uncertain', 'definitely_failed']) {
      expect(validate({ ...examples.dispatch, evidence })).toBe(true);
    }
    for (const patch of [
      { evidence: 'success' },
      { evidence: 'running' },
      { pane: '%14' },
      { error: { code: 'NATIVE_ERROR', message: 'private details' } },
      { requestId: 'local-database-identifier' },
    ]) {
      expect(validate({ ...examples.dispatch, ...patch })).toBe(false);
    }
  });
});

describe('Office design-vector integrity (not behavioral enforcement)', () => {
  it('keeps unique, owned and actionable vectors with valid example references', () => {
    expect(scenarios.cases.length).toBeGreaterThan(0);
    expect(new Set(scenarios.cases.map((scenario) => scenario.id)).size).toBe(
      scenarios.cases.length
    );
    for (const scenario of scenarios.cases) {
      expect(scenario.issue).toBeGreaterThanOrEqual(176);
      expect(scenario.issue).toBeLessThanOrEqual(182);
      expect(scenario.given.length).toBeGreaterThan(0);
      expect(scenario.when.length).toBeGreaterThan(0);
      expect(scenario.then.length).toBeGreaterThan(0);
      if (scenario.example) expect(Object.hasOwn(examples, scenario.example)).toBe(true);
    }
  });
});
