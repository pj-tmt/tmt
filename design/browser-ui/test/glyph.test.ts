import { expect, it } from 'vite-plus/test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { Info, Link, User } from 'lucide-react';
import { moduleIcon, shellIconNodes } from '../src/shell-icons.js';

it('shares the pinned Lucide Link, Info and User nodes without extending module admission', () => {
  for (const [name, icon] of [
    ['link', Link],
    ['info', Info],
    ['user', User],
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
