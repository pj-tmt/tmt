import { expect, it } from 'vite-plus/test';
import { assertUiUtilities, uiCandidates } from '../scripts/ui-source-policy.mjs';

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

it('allows the named radius tokens, the elevation levels and token-backed motion, nothing else', () => {
  for (const source of [
    'className="tmt:rounded-control"',
    'className="tmt:rounded-small"',
    'className="tmt:rounded-surface"',
    'className="tmt:rounded-pill"',
    'className="tmt:shadow-overlay"',
    'className="tmt:shadow-elevation-1"',
    'className="tmt:shadow-elevation-3"',
    'className="tmt:duration-(--tmt-foundation-motion-fast) tmt:ease-(--tmt-foundation-motion-ease-out)"',
    'className="tmt:data-[state=open]:animate-[tmt-foundation-pop-in_var(--tmt-foundation-motion-fast)_var(--tmt-foundation-motion-ease-out)_both]"',
  ])
    expect(() => uiCandidates(source), source).not.toThrow();
  for (const source of [
    'className="tmt:rounded"',
    'className="tmt:rounded-full"',
    'className="tmt:rounded-lg"',
    'className="tmt:shadow"',
    'className="tmt:shadow-lg"',
    'className="tmt:ring"',
    'className="tmt:ring-2"',
    'className="tmt:hover:shadow-md"',
    'className="tmt:duration-200"',
    'className="tmt:duration-[150ms]"',
    'className="tmt:delay-100"',
    'className="tmt:ease-in-out"',
    'className="tmt:ease-[cubic-bezier(0,0,1,1)]"',
    'className="tmt:animate-[spin_1s_linear_infinite]"',
    'className="tmt:shadow-elevation-2"',
  ])
    expect(() => uiCandidates(source), source).toThrow();
});

it('allows namespaced font-body identifiers but rejects real global output and element selectors', () => {
  for (const css of [
    '.tmt\\:font-body { font-family: var(--tmt-foundation-font-body); }',
    '.body { color: inherit; }',
    '.tmt\\:body { color: inherit; }',
  ])
    expect(() => assertUiUtilities(css), css).not.toThrow();
  for (const css of [
    'body { margin: 0; }',
    'html { color: red; }',
    'body:hover { color: red; }',
    'html[data-theme] { color: red; }',
    'html, body { color: red; }',
    '.tmt\\:parent { & body { color: red; } }',
    ':root { --host: red; }',
    '@property --host { syntax: "*"; inherits: true; }',
    '@import "host.css";',
    '@layer theme { * { color: red; } }',
  ])
    expect(() => assertUiUtilities(css), css).toThrow('Unexpected global output');
});

it('retains quotes inside stock attribute variants without truncating the utility', () => {
  const candidate = "tmt:[&_svg:not([class*='size-'])]:size-4";
  expect(uiCandidates(`className="${candidate} tmt:p-2"`)).toEqual([candidate, 'tmt:p-2']);
  for (const source of [
    'className="tmt:[data-state=ready"',
    'className={"tmt:" + value}',
    'className={`tmt:p-${size}`}',
  ])
    expect(() => uiCandidates(source)).toThrow('complete literal');
});
