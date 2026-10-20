import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { expect, it } from 'vite-plus/test';
import { MarkdownText } from '../src/components/markdown-text.js';
import {
  classifyHref,
  MAX_DEPTH,
  MAX_SOURCE,
  parseMarkdown,
  type Block,
  type Inline,
} from '../src/markdown.js';

const origin = 'https://colab.example';
const parse = (source: string) => parseMarkdown(source, origin);
const render = (value: string, names: string[] = []) =>
  renderToStaticMarkup(createElement(MarkdownText, { value, names, origin }));
/** The text a tree shows, markers removed. */
const words = (nodes: readonly Inline[]): string =>
  nodes
    .map((node) =>
      node.kind === 'text' || node.kind === 'code' ? node.value : words(node.children),
    )
    .join('');
const paragraph = (block: Block | undefined) => {
  expect(block?.kind).toBe('paragraph');
  return block as Extract<Block, { kind: 'paragraph' }>;
};

it('keeps plain text as paragraphs, with newlines inside a paragraph kept as text', () => {
  const tree = parse('first line\nsecond line\n\nnext paragraph');
  expect(tree).toHaveLength(2);
  expect(words(paragraph(tree[0]).children)).toBe('first line\nsecond line');
  expect(words(paragraph(tree[1]).children)).toBe('next paragraph');
  expect(parse('')).toEqual([]);
  expect(parse('\r\none\r\ntwo')).toEqual(parse('one\ntwo'));
});

it('renders emphasis, strong text, inline code and escapes', () => {
  expect(render('a **bold** and *em* and `code` and \\*literal\\*')).toBe(
    '<div class="markdown-body "><p>a <strong>bold</strong> and <em>em</em> and <code>code</code> and *literal*</p></div>',
  );
  // Unmatched or word-internal markers stay literal.
  expect(words(paragraph(parse('snake_case_name and 2 * 3 * 4 and **open')[0]).children)).toBe(
    'snake_case_name and 2 * 3 * 4 and **open',
  );
  expect(words(paragraph(parse('``a ` b``')[0]).children)).toBe('a ` b');
});

it('renders fenced code as text, lists, numbered lists and quotes', () => {
  const fenced = parse('```ts\nconst a = "<b>";\n*not em*\n```\nafter');
  expect(fenced[0]).toEqual({ kind: 'code', value: 'const a = "<b>";\n*not em*' });
  expect(words(paragraph(fenced[1]).children)).toBe('after');
  // An unterminated fence runs to the end of the body.
  expect(parse('```\nopen')[0]).toEqual({ kind: 'code', value: 'open' });
  // The blank lines kept between blocks are layout-neutral whitespace; compare without them.
  const html = render(
    '- one\n- two\n  - nested\n\n3. third\n4. fourth\n\n> quoted\n> text',
  ).replaceAll('>\n\n<', '><');
  expect(html).toContain(
    '<ul><li><p>one</p></li><li><p>two</p><ul><li><p>nested</p></li></ul></li></ul>',
  );
  expect(html).toContain('<ol start="3"><li><p>third</p></li><li><p>fourth</p></li></ol>');
  expect(html).toContain('<blockquote><p>quoted\ntext</p></blockquote>');
});

it('keeps unsupported syntax as the text the author wrote', () => {
  const source = '# Heading\n\n| a | b |\n|---|---|\n\n![image](https://example.com/x.png)\n\n---';
  const tree = parse(source);
  expect(tree.every((block) => block.kind === 'paragraph')).toBe(true);
  expect(render(source)).not.toContain('<h1');
  expect(render(source)).not.toContain('<img');
  expect(render(source)).toContain('# Heading');
});

it('opens same-space page links and keeps outside links inert', () => {
  expect(classifyHref('/p/deadbeef', origin)).toEqual({ kind: 'page', href: '/p/deadbeef' });
  expect(classifyHref('/p/deadbeef#section-1', origin)).toEqual({
    kind: 'page',
    href: '/p/deadbeef#section-1',
  });
  expect(classifyHref(`${origin}/p/deadbeef#a`, origin)).toEqual({
    kind: 'page',
    href: '/p/deadbeef#a',
  });
  expect(classifyHref('https://example.com/a?b=1', origin)).toEqual({
    kind: 'outside',
    href: 'https://example.com/a?b=1',
  });
  expect(classifyHref('http://localhost:3000/x', origin).kind).toBe('outside');
  const html = render('[page](/p/deadbeef) and [site](https://example.com/a)');
  expect(html).toContain('<a class="markdown-link" href="/p/deadbeef">page</a>');
  // An outside link is the words and the URL, never an anchor.
  expect(html).toContain('site (https://example.com/a)');
  expect(html.match(/<a /g)).toHaveLength(1);
});

it('never turns an unsafe link target into a link', () => {
  const unsafe = [
    'javascript:alert(1)',
    'JaVaScRiPt:alert(1)',
    'jav\tascript:alert(1)',
    'jav\nascript:alert(1)',
    '&#106;avascript:alert(1)',
    'data:text/html,<script>alert(1)</script>',
    'file:///etc/passwd',
    'colab-attachment:abc',
    'mailto:someone@example.com',
    'vbscript:x',
    '//evil.example/x',
    '/\\evil.example',
    '/p/not-a-prefix',
    'https://user:secret@example.com/',
    'http://example.com/',
    'ftp://example.com/',
    ' https://example.com/',
    '',
  ];
  for (const target of unsafe) {
    expect(classifyHref(target, origin), JSON.stringify(target)).toEqual({ kind: 'none' });
    const html = render(`[label](${target})`);
    expect(html, JSON.stringify(target)).not.toContain('<a ');
    expect(html, JSON.stringify(target)).not.toContain('href=');
  }
  // The text the author wrote is kept.
  expect(words(paragraph(parse('[x](javascript:alert(1))')[0]).children)).toBe(
    '[x](javascript:alert(1))',
  );
});

it('treats HTML, entities and markup in text and link labels as text', () => {
  const html = render(
    '<script>alert(1)</script> <img src=x onerror=alert(1)> &lt;script&gt; &#60; ' +
      '[<b>x</b>](/p/deadbeef) **<i>y</i>** `<u>z</u>`',
  );
  expect(html).not.toMatch(/<(script|img|b|i|u)[ >]/);
  expect(html).toContain('&lt;script&gt;alert(1)&lt;/script&gt;');
  expect(html).toContain('&amp;lt;script&amp;gt; &amp;#60;');
  // A fenced block cannot close the markup around it.
  const fenced = render('```\n</pre><script>alert(1)</script>\n```');
  expect(fenced).not.toContain('<script');
  expect(fenced).toContain('&lt;/pre&gt;&lt;script&gt;');
});

it('keeps mentions working in text and inside emphasis, not inside code', () => {
  const html = render('ask **@Atlas** or `@Atlas` or @Atlas', ['Atlas']);
  expect(html.match(/agent-chip"/g)).toHaveLength(2);
  expect(html).toMatch(/<strong><span class="agent-chip"/);
  expect(html).toContain('<code>@Atlas</code>');
});

it('falls back to plain text for a body that is too large, too deep or too busy', () => {
  const big = `${'word '.repeat(MAX_SOURCE / 5)}**x**`;
  expect(big.length).toBeGreaterThan(MAX_SOURCE);
  const plain = parse(big);
  expect(plain.every((block) => block.kind === 'paragraph')).toBe(true);
  expect(words(paragraph(plain[0]).children)).toContain('**x**');
  // Nested quotes and lists stop at the depth cap, keeping the text.
  const quotes = parse(`${'> '.repeat(MAX_DEPTH + 3)}deep`);
  expect(JSON.stringify(quotes)).toContain('deep');
  const lists = parse(`${'  '.repeat(0)}- a\n  - b\n    - c\n      - d\n        - e deep`);
  expect(JSON.stringify(lists)).toContain('e deep');
  // Thousands of tiny nodes are over the node budget and stay plain.
  const busy = parse('*a* '.repeat(2000));
  expect(busy.every((block) => block.kind === 'paragraph')).toBe(true);
  expect(JSON.stringify(busy)).not.toContain('emphasis');
  // 5,000 markers in a row neither hang nor throw.
  expect(() => parse('*'.repeat(5000))).not.toThrow();
  expect(() => parse('['.repeat(5000))).not.toThrow();
  expect(() => parse('`'.repeat(5000))).not.toThrow();
  expect(() => parse('> '.repeat(2000))).not.toThrow();
});

it('keeps blank lines between blocks in the text so a copied body keeps its paragraphs', () => {
  const html = render('one\n\ntwo');
  expect(html.replace(/<[^>]+>/g, '')).toBe('one\n\ntwo');
});
