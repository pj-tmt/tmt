import { readFile, readdir, writeFile } from 'node:fs/promises';
import { compile } from 'tailwindcss';
import { format } from 'vite-plus/fmt';
import { assertUiUtilities, uiCandidates } from './ui-source-policy.mjs';

const home = new URL('../', import.meta.url);
const tokens = JSON.parse(
  await readFile(new URL('../../tokens/tokens.json', import.meta.url), 'utf8'),
);
const mode = process.argv[2];
if (process.argv.length !== 3 || !['--write', '--check'].includes(mode))
  throw new Error('Use --write or --check');

// No theme values are copied here: these are role aliases into the sole token document.
const roles = {
  background: ['surface', 'paper'],
  foreground: ['color', 'text'],
  card: ['surface', 'sheet'],
  'card-foreground': ['color', 'text'],
  popover: ['surface', 'sheet'],
  'popover-foreground': ['color', 'text'],
  primary: ['color', 'accent'],
  'primary-foreground': ['color', 'on-accent'],
  secondary: ['surface', 'command'],
  'secondary-foreground': ['color', 'text'],
  muted: ['surface', 'disabled'],
  'muted-foreground': ['color', 'secondary'],
  accent: ['surface', 'selection'],
  'accent-foreground': ['color', 'selected-text'],
  destructive: ['color', 'blocked'],
  'destructive-ring': ['color', 'focus-destructive'],
  'destructive-soft': ['surface', 'blocked-soft'],
  'layer-boundary': ['surface', 'rule'],
  border: ['surface', 'rule'],
  input: ['surface', 'rule'],
  indicator: ['color', 'edge'],
  ring: ['color', 'focus'],
  scrim: ['surface', 'scrim'],
  // Not colour utilities: the elevation scale (#5821). `shadow-overlay` is the floating layers' level.
  'shadow-overlay': ['shadow', 'overlay'],
  'shadow-elevation-1': ['shadow', 'elevation-1'],
  'shadow-elevation-3': ['shadow', 'elevation-3'],
};
const colourRoles = Object.keys(roles).filter((key) => !key.startsWith('shadow-'));
/** The motion tokens as foundation properties; `reduced` zeroes every duration. */
function motion(reduced = false, indent = '  ') {
  return Object.entries(tokens.browser.motion)
    .filter(
      ([key]) => !key.startsWith('$') && (!reduced || ['fast', 'base', 'pulse'].includes(key)),
    )
    .map(([key, value]) => `${indent}--tmt-foundation-motion-${key}: ${reduced ? '0ms' : value};`);
}
const themed = (theme) =>
  Object.entries(roles)
    .map(
      ([name, [group, role]]) =>
        `  --tmt-foundation-${name}: ${tokens.browser[group][role][theme].toLowerCase()};`,
    )
    .join('\n');
// Palette B (#5821): inside [data-tmt-palette='b'] the foundation reads the leaf's semantic roles, which
// static.css remaps there, so the set follows B, the viewer theme and the radius knob with no values here.
const paletteB = [
  ...Object.entries(roles).map(
    ([name, [group, role]]) => `  --tmt-foundation-${name}: var(--tmt-ui-${group}-${role});`,
  ),
  ...['radius', 'radius-s', 'radius-l'].map(
    (key) => `  --tmt-foundation-${key}: var(--tmt-ui-${key});`,
  ),
].join('\n');
const fixed = [
  ...Object.entries(tokens.browser.ui).map(
    ([key, value]) => `  --tmt-foundation-${key}: ${value};`,
  ),
  ...[
    'radius',
    'radius-s',
    'radius-l',
    'radius-pill',
    'rule-width',
    'focus-width',
    'command-size',
    'caption-size',
    'eyebrow-weight',
  ].map((key) => `  --tmt-foundation-${key}: ${tokens.browser.metric[key]};`),
  ...motion(),
  ...Object.entries(tokens.font).map(
    ([key, value]) => `  --tmt-foundation-font-${key}: ${value.stack};`,
  ),
].join('\n');
async function sources(directory) {
  return (
    await Promise.all(
      (await readdir(directory, { withFileTypes: true }))
        .sort((a, b) => a.name.localeCompare(b.name, 'en'))
        .map(async (entry) => {
          const file = new URL(entry.name, directory);
          if (entry.isDirectory()) return sources(new URL(`${entry.name}/`, directory));
          return /\.[jt]sx?$/.test(entry.name) ? readFile(file, 'utf8') : '';
        }),
    )
  )
    .flat()
    .join('\n');
}
const source = (
  await Promise.all(['src/ui/', 'test/fixtures/ui/'].map((path) => sources(new URL(path, home))))
).join('\n');
const candidates = uiCandidates(source);
// Layer enter and exit as named animations (`animate-pop-in`, `animate-slide-out-right`, ...) over
// the keyframes below, on the motion tokens: `pop-fast-*` is the fast duration, the rest the base.
// #5821: entering takes the base duration with ease-out (a floating layer the fast one), leaving the
// fast one with ease-in, and a floating layer leaves in two thirds of its entry, so exit is quicker.
const motionVar = (token) => `var(--tmt-foundation-motion-${token})`;
const durations = {
  fast: motionVar('fast'),
  base: motionVar('base'),
  quick: `calc(${motionVar('fast')} * 2 / 3)`,
};
const animations = [
  ['pop-fast-in', 'pop-in', 'fast', 'ease-out'],
  ['pop-fast-out', 'pop-out', 'fast', 'ease-in'],
  ['pop-in', 'pop-in', 'base', 'ease-out'],
  ['pop-out', 'pop-out', 'base', 'ease-in'],
  ['fade-in', 'fade-in', 'base', 'ease-out'],
  ['fade-out', 'fade-out', 'base', 'ease-in'],
  ['fade-fast-out', 'fade-out', 'fast', 'ease-in'],
  ['fade-quick-out', 'fade-out', 'quick', 'ease-in'],
  ...['right', 'left', 'top', 'bottom'].flatMap((side) => [
    [`slide-in-${side}`, `slide-in-${side}`, 'base', 'ease-out'],
    [`slide-out-${side}`, `slide-out-${side}`, 'fast', 'ease-in'],
    [`rise-in-${side}`, `rise-in-${side}`, 'fast', 'ease-out'],
  ]),
  ['rise-in', 'rise-in-top', 'base', 'ease-out'],
]
  .map(
    ([name, keyframes, duration, ease]) =>
      `--animate-${name}: tmt-foundation-${keyframes} ${durations[duration]} ${motionVar(`ease-${ease.slice(5)}`)} both;`,
  )
  .concat(
    `--animate-pulse-calm: tmt-foundation-pulse ${motionVar('pulse')} ${motionVar('ease-in')} infinite alternate;`,
  );
const theme = [
  '--*: initial;',
  ...colourRoles.map((key) => `--color-${key}: var(--tmt-foundation-${key});`),
  '--shadow-overlay: var(--tmt-foundation-shadow-overlay);',
  '--shadow-elevation-1: var(--tmt-foundation-shadow-elevation-1);',
  '--shadow-elevation-3: var(--tmt-foundation-shadow-elevation-3);',
  '--radius-control: var(--tmt-foundation-radius);',
  '--radius-small: var(--tmt-foundation-radius-s);',
  '--radius-surface: var(--tmt-foundation-radius-l);',
  '--radius-pill: var(--tmt-foundation-radius-pill);',
  '--spacing: var(--tmt-foundation-spacing-unit);',
  ...animations,
  ...Object.keys(tokens.font).map((key) => `--font-${key}: var(--tmt-foundation-font-${key});`),
].join('\n');
// No imports, default theme or preflight. Only this leaf's complete prefixed class literals.
const compiler = await compile(`@theme inline prefix(tmt) {\n${theme}\n}\n@tailwind utilities;`);
let utilities = compiler.build(candidates);
// Tailwind registers/reset its internal properties globally. Instead reset them locally on
// every island element/pseudo-element (the equivalent non-inheriting defaults), and never
// publish @property registrations or the universal fallback layer into the host document.
const defaults = [];
utilities = utilities.replace(/@property (--tw-[\w-]+) \{([^}]*)\}\n/g, (_, key, body) => {
  defaults.push(`    ${key}: ${body.match(/initial-value: ([^;]+);/)?.[1] ?? 'initial'};`);
  return '';
});
const fallback = utilities.indexOf('@layer properties {');
if (fallback !== -1) utilities = utilities.slice(0, fallback);
utilities = utilities
  .replace('@layer properties;\n', '')
  .replaceAll('--tw-', '--tmt-foundation-tw-');
assertUiUtilities(utilities);
const reset = defaults.length
  ? `  &, & *, &::before, &::after, & *::before, & *::after, & *::backdrop {\n${defaults.join('\n').replaceAll('--tw-', '--tmt-foundation-tw-')}\n  }\n`
  : '';
// Layer enter and exit, named in the layer classes (`animate-[tmt-foundation-pop-in_...]`). Radix
// keeps a layer mounted while its exit animation runs. `scale` and `transform` compose with the
// `translate` a layer already uses for its position.
const slides = ['right', 'left', 'top', 'bottom'].map((side) => {
  const from = {
    right: 'translateX(100%)',
    left: 'translateX(-100%)',
    top: 'translateY(-100%)',
    bottom: 'translateY(100%)',
  }[side];
  return `@keyframes tmt-foundation-slide-in-${side} {\n  from {\n    transform: ${from};\n  }\n}\n@keyframes tmt-foundation-slide-out-${side} {\n  to {\n    transform: ${from};\n  }\n}`;
});
// A floating layer rises a short way from the side it opens toward: a layer placed below its anchor
// comes down to it, one placed above comes up, and so on.
const rises = ['right', 'left', 'top', 'bottom'].map((side) => {
  const rise = 'var(--tmt-foundation-motion-rise)';
  const from = {
    right: `calc(${rise} * -1) 0`,
    left: `${rise} 0`,
    top: `0 ${rise}`,
    bottom: `0 calc(${rise} * -1)`,
  }[side];
  return `@keyframes tmt-foundation-rise-in-${side} {\n  from {\n    opacity: 0;\n    translate: ${from};\n  }\n}`;
});
const keyframes = [
  '@keyframes tmt-foundation-fade-in {\n  from {\n    opacity: 0;\n  }\n}',
  '@keyframes tmt-foundation-fade-out {\n  to {\n    opacity: 0;\n  }\n}',
  '@keyframes tmt-foundation-pop-in {\n  from {\n    opacity: 0;\n    scale: var(--tmt-foundation-motion-scale);\n  }\n}',
  '@keyframes tmt-foundation-pop-out {\n  to {\n    opacity: 0;\n    scale: var(--tmt-foundation-motion-scale);\n  }\n}',
  ...slides,
  ...rises,
  '@keyframes tmt-foundation-pulse {\n  to {\n    opacity: 0.55;\n  }\n}',
].join('\n');
const css = `/* Generated by scripts/generate-ui-css.mjs; no preflight or global theme. */
[data-tmt-ui] {\n${fixed}\n${themed('light')}\n  color-scheme: light;\n}
@media (prefers-color-scheme: dark) {
  [data-tmt-ui]:not([data-tmt-theme='light']) {\n${themed('dark')}\n    color-scheme: dark;\n  }
}
[data-tmt-ui][data-tmt-theme='light'] {\n${themed('light')}\n  color-scheme: light;\n}
[data-tmt-ui][data-tmt-theme='dark'] {\n${themed('dark')}\n  color-scheme: dark;\n}
[data-tmt-palette='b'] [data-tmt-ui],
[data-tmt-ui][data-tmt-palette='b'] {\n${paletteB}\n}
/* Under palette B a disabled button of any variant is neutral grey, never a faded accent (#5821). */
[data-tmt-palette='b'] [data-tmt-ui] [data-slot='button']:disabled,
[data-tmt-ui][data-tmt-palette='b'] [data-slot='button']:disabled {
  border-color: var(--tmt-foundation-border);
  background: var(--tmt-foundation-muted);
  color: var(--tmt-foundation-muted-foreground);
  opacity: 1;
}
@media (prefers-reduced-motion: reduce) {
  [data-tmt-ui] {\n${motion(true, '    ').join('\n')}\n  }
}
${keyframes}
:where([data-tmt-ui]) {\n${reset}${utilities
  .trim()
  .split('\n')
  .map((line) => `  ${line}`)
  .join('\n')}\n}\n`;
const formatted = await format('generated/ui.css', css, { singleQuote: true, printWidth: 100 });
if (formatted.errors.length) throw new Error(JSON.stringify(formatted.errors));
const output = new URL('generated/ui.css', home);
if (mode === '--write') await writeFile(output, formatted.code, 'utf8');
else if ((await readFile(output, 'utf8')) !== formatted.code)
  throw new Error('Generated ui.css is stale');
