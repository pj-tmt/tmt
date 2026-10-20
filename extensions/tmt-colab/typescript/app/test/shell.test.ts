import { expect, it } from 'vite-plus/test';
import vectors from '../../../contracts/vectors/shell-v1.json';
import { canonical } from '../src/closed.js';
import { applyShellAction, validateShell, type Shell } from '../src/shell.js';
import type { Attribution } from '../src/summary-lines.js';

it.each(vectors.cases)('$name', (fixture) => {
  const before = JSON.stringify(fixture.document);
  const run = () =>
    'action' in fixture
      ? applyShellAction(
          fixture.document,
          fixture.action!,
          fixture.body,
          fixture.attribution as Attribution,
        )
      : validateShell(fixture.document);
  if (!fixture.valid) expect(run).toThrow('Invalid structured value');
  else {
    const result = run();
    if ('expected' in fixture) expect(result).toEqual(fixture.expected);
    expect(canonical(result ?? fixture.document)).toBe(fixture.canonical);
  }
  expect(JSON.stringify(fixture.document)).toBe(before);
});

it('enforces the whole-value byte limit at exactly 128 KiB and the 256-item bound', () => {
  const value: Shell = { version: 1, items: {}, order: [] };
  for (let n = 0; n < 256; n++) {
    const id = `10000000-0000-4000-8000-${n.toString(16).padStart(12, '0')}`;
    value.items[id] = { kind: 'page', pageId: '20000000-0000-4000-8000-000000000001' };
    value.order.push(id);
  }
  expect(() => validateShell(value)).not.toThrow();
  const extra = '10000000-0000-4000-8000-ffffffffffff';
  const overItems = structuredClone(value);
  overItems.items[extra] = value.items[value.order[0]];
  overItems.order.push(extra);
  expect(() => validateShell(overItems)).toThrow();
  let last = '';
  for (const id of value.order) {
    value.items[id].note = '';
    const remaining = 131072 - new TextEncoder().encode(canonical(value)).length;
    value.items[id].note = 'x'.repeat(Math.min(512, remaining));
    if (remaining <= 512) {
      last = id;
      break;
    }
  }
  expect(new TextEncoder().encode(canonical(value)).length).toBe(131072);
  expect(() => validateShell(value)).not.toThrow();
  value.items[last].note += 'x';
  expect(() => validateShell(value)).toThrow();
});
