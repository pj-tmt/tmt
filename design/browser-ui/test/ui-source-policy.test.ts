import { expect, it } from 'vite-plus/test';
import { uiCandidates } from '../scripts/ui-source-policy.mjs';

it('requires literal prefixed, square, token-backed utilities instead of stock styling', () => {
  expect(uiCandidates('className="tmt:bg-background tmt:p-2"')).toEqual([
    'tmt:bg-background',
    'tmt:p-2',
  ]);
  for (const source of [
    'className="bg-background"',
    'className="tmt:rounded-md"',
    'className="tmt:shadow-md"',
    'className="tmt:focus:ring-2"',
    'className="tmt:bg-[#fff]"',
    'className="tmt:text-red-500"',
    'className="tmt:bg-[rgb(0,0,0)]"',
    'className="tmt:bg-[blue]"',
    'className="tmt:bg-(--other-palette)"',
    'className="tmt:[border-radius:12px]"',
  ])
    expect(() => uiCandidates(source), source).toThrow();
});
