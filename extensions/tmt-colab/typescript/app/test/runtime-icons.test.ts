import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { expect, it } from 'vite-plus/test';
import { icons } from '../src/runtime/icons.js';

const render = (name: string, props: Record<string, unknown> = {}) =>
  renderToStaticMarkup(createElement(icons[name as keyof typeof icons], props));

it('lists each icon once, in order, as a frozen set of components', () => {
  const names = Object.keys(icons);
  expect(names.length).toBeGreaterThan(100);
  expect(names).toEqual([...new Set(names)].sort());
  for (const name of names) expect(name).toMatch(/^[A-Z][A-Za-z0-9]*$/);
  expect(Object.isFrozen(icons)).toBe(true);
  for (const name of names) expect(render(name)).toMatch(/^<svg /);
});

it('draws square caps, miter joins, currentColor, stroke 1.75 and size 16 by default', () => {
  const svg = render('Check');
  for (const part of [
    'width="16"',
    'height="16"',
    'stroke="currentColor"',
    'stroke-width="1.75"',
    'stroke-linecap="square"',
    'stroke-linejoin="miter"',
  ])
    expect(svg).toContain(part);
});

it('lets the page override every default, and treats undefined as not passed', () => {
  const svg = render('Check', {
    size: 24,
    color: 'red',
    strokeWidth: 2,
    strokeLinecap: 'round',
    strokeLinejoin: 'round',
  });
  for (const part of [
    'width="24"',
    'stroke="red"',
    'stroke-width="2"',
    'stroke-linecap="round"',
    'stroke-linejoin="round"',
  ])
    expect(svg).toContain(part);
  const unset = render('Check', { size: undefined, strokeWidth: undefined });
  expect(unset).toContain('width="16"');
  expect(unset).toContain('stroke-width="1.75"');
});

it('hides an unnamed icon from assistive technology and exposes a named one as an image', () => {
  const plain = render('Check');
  expect(plain).toContain('aria-hidden="true"');
  expect(plain).not.toContain('role=');

  const labelled = render('Check', { 'aria-label': 'Done' });
  expect(labelled).toContain('role="img"');
  expect(labelled).toContain('aria-label="Done"');
  expect(labelled).not.toContain('aria-hidden');

  const titled = render('Check', { title: 'Saved' });
  expect(titled).toContain('role="img"');
  expect(titled).toContain('<title>Saved</title>');
  expect(titled).not.toContain('aria-hidden');
  expect(titled).not.toContain('title="');

  const own = render('Check', { 'aria-label': 'Done', role: 'presentation' });
  expect(own).toContain('role="presentation"');
  expect(own).not.toContain('role="img"');
});
