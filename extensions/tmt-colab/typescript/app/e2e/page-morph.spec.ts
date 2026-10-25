import { readFileSync } from 'node:fs';
import { expect, test } from '@playwright/test';

/**
 * The in-place update's morph (#5022), one table of node cases. It runs the exact code of the
 * PAGE MORPH region of renderer.html in a blank page: `before` is the live body, `after` the new
 * source, `expected` the body afterwards. `kept` names the ids whose element must be the same node
 * (an in-place update keeps scroll, focus and state only on the nodes it keeps).
 */
const renderer = readFileSync(new URL('../public/renderer.html', import.meta.url), 'utf8');
const from = renderer.indexOf('// BEGIN PAGE MORPH');
const to = renderer.indexOf('// END PAGE MORPH');
if (from < 0 || to <= from) throw new Error('Missing renderer page morph markers');
const region = renderer.slice(from, to);

interface MorphCase {
  name: string;
  before: string;
  after: string;
  expected?: string;
  kept: string[];
  /** Applied to the live body before the morph: what the frame or the reader did to it. */
  live?: string;
}
const cases: MorphCase[] = [
  {
    name: 'text change',
    before: '<p id="a">one</p><p id="b">two</p>',
    after: '<p id="a">uno</p><p id="b">two</p>',
    kept: ['a', 'b'],
  },
  {
    name: 'insert',
    before: '<p id="a">A</p><p id="c">C</p>',
    after: '<p id="a">A</p><p id="b">B</p><p id="c">C</p>',
    kept: ['a', 'c'],
  },
  {
    name: 'remove',
    before: '<p id="a">A</p><p id="b">B</p><p id="c">C</p>',
    after: '<p id="a">A</p><p id="c">C</p>',
    kept: ['a', 'c'],
  },
  {
    name: 'reorder by id',
    before: '<p id="a">A</p><p id="b">B</p><p id="c">C</p>',
    after: '<p id="c">C</p><p id="a">A</p><p id="b">B</p>',
    kept: ['a', 'b', 'c'],
  },
  {
    name: 'attribute change',
    before: '<section id="s"><p id="a" class="x" title="t">A</p></section>',
    after: '<section id="s"><p id="a" class="y">A</p></section>',
    kept: ['s', 'a'],
  },
  {
    name: 'owner-marked skip',
    before: '<p id="a">A</p><p id="b">B</p><p id="c" data-choice="">C</p>',
    // A frame spacer after #a, a frame attribute on #b, and a choice the frame hid.
    live: 'owned',
    after: '<p id="a">A changed</p><p id="b">B</p><p id="c" data-choice="">C</p>',
    expected:
      '<p id="a">A changed</p><div data-colab-proposal-slot=""></div><p id="b" data-colab-question="">B</p><p id="c" data-choice="" hidden="">C</p>',
    kept: ['a', 'b', 'c'],
  },
  {
    name: 'expanded element compares its own attributes only',
    before: '<p id="a">A <tmt-icon id="i" name="check"></tmt-icon></p>',
    live: 'icon',
    after: '<p id="a">A, edited <tmt-icon id="i" name="x"></tmt-icon></p>',
    expected:
      '<p id="a">A, edited <tmt-icon id="i" name="x"><svg data-drawn=""></svg></tmt-icon></p>',
    kept: ['a', 'i'],
  },
  {
    name: 'reader state stays until its source attribute changes',
    before:
      '<details id="d"><summary>S</summary>T</details><input id="t" value="one"><input id="u" value="one">',
    live: 'reader',
    after:
      '<details id="d"><summary>S</summary>T, edited</details><input id="t" value="one"><input id="u" value="two">',
    expected:
      '<details id="d" open=""><summary>S</summary>T, edited</details><input id="t" value="one"><input id="u" value="two">',
    kept: ['d', 't', 'u'],
  },
];

test('the page morph table', async ({ page }) => {
  await page.setContent('<!doctype html><html><body></body></html>');
  const results = await page.evaluate(
    ({ region, cases }) => {
      const { morphPage, rememberSource } = new Function(
        `${region}\nreturn { morphPage, rememberSource };`,
      )() as {
        morphPage(live: Element, next: Element, owner: unknown): void;
        rememberSource(root: Element, state: WeakMap<Element, unknown>): void;
      };
      return cases.map((item) => {
        const body = document.body;
        body.innerHTML = item.before;
        const state = new WeakMap<Element, unknown>();
        rememberSource(body, state);
        const owned = new WeakSet<Node>();
        const hidden = new WeakSet<Element>();
        const values: Record<string, unknown> = {};
        if (item.live === 'owned') {
          const spacer = document.createElement('div');
          spacer.setAttribute('data-colab-proposal-slot', '');
          owned.add(spacer);
          body.querySelector('#a')!.after(spacer);
          body.querySelector('#b')!.setAttribute('data-colab-question', '');
          body.querySelector('#c')!.setAttribute('hidden', '');
          hidden.add(body.querySelector('#c')!);
        }
        if (item.live === 'icon') {
          const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
          svg.setAttribute('data-drawn', '');
          body.querySelector('#i')!.append(svg);
        }
        if (item.live === 'reader') {
          (body.querySelector('#d') as HTMLDetailsElement).open = true;
          (body.querySelector('#t') as HTMLInputElement).value = 'typed';
          (body.querySelector('#u') as HTMLInputElement).value = 'typed';
        }
        const nodes = new Map(item.kept.map((id) => [id, body.querySelector(`#${id}`)]));
        const next = new DOMParser().parseFromString(
          `<!doctype html><html><head></head><body>${item.after}</body></html>`,
          'text/html',
        ).body;
        morphPage(body, next, {
          node: (node: Node) => owned.has(node),
          attribute: (el: Element, name: string) =>
            name.startsWith('data-colab-') || (name === 'hidden' && hidden.has(el)),
          expanded: (el: Element) => el.localName === 'tmt-icon',
          state,
        });
        if (item.live === 'reader') {
          values.t = (body.querySelector('#t') as HTMLInputElement).value;
          values.u = (body.querySelector('#u') as HTMLInputElement).value;
        }
        return {
          name: item.name,
          html: body.innerHTML,
          kept: item.kept.filter((id) => nodes.get(id) === body.querySelector(`#${id}`)),
          values,
        };
      });
    },
    { region, cases },
  );
  for (const [index, item] of cases.entries()) {
    const result = results[index];
    expect(result.html, item.name).toBe(item.expected ?? item.after);
    expect(result.kept, item.name).toEqual(item.kept);
  }
  // A typed value stays while its source attribute stays; a changed attribute replaces it.
  expect(results.at(-1)!.values).toEqual({ t: 'typed', u: 'two' });
});
