import { expect, it } from 'vite-plus/test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { Info, Link } from 'lucide-react';
import { moduleIcon, shellIconNodes } from '../src/shell-icons.js';

it('shares the pinned Lucide Link and Info nodes without extending module admission', () => {
  for (const [name, icon] of [
    ['link', Link],
    ['info', Info],
  ] as const) {
    const svg = renderToStaticMarkup(createElement(icon));
    const nodes = Array.from(svg.matchAll(/<(path|circle) ([^>]+)>/g), (match) => [
      match[1],
      Object.fromEntries(
        Array.from(match[2]!.matchAll(/([\w-]+)="([^"]*)"/g), (attribute) => [
          attribute[1],
          attribute[2],
        ]),
      ),
    ]);
    expect(shellIconNodes[name]).toEqual(nodes);
    expect(moduleIcon(name)).toBe('box');
  }
});
