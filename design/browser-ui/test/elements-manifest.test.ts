import { expect, it } from 'vite-plus/test';
import {
  browserElements,
  browserElement,
  elementAttributeIssue,
  mayUseBrowserElements,
} from '../src/elements-manifest.js';
import { browserCatalog } from '../src/catalog.js';

it('indexes every registered element and every state by the same manifest', () => {
  expect(new Set(browserElements.map((entry) => entry.tag)).size).toBe(browserElements.length);
  for (const entry of browserElements) {
    expect(browserElement(entry.tag)).toBe(entry);
    expect(entry.examples.length).toBeGreaterThan(0);
    const catalog = browserCatalog.find((item) => item.tag === entry.tag)!;
    expect(catalog.examples).toBe(entry.examples);
    expect(catalog.coverage.tests).toHaveLength(entry.examples.length * 4);
    expect(catalog.coverage.test).toBe(true);
    for (const example of entry.examples) expect(mayUseBrowserElements(example.html)).toBe(true);
  }
  expect(browserElement('tmt-ui-unknown')).toBeUndefined();
  expect(mayUseBrowserElements('<p class="tmt-ui-card">Text</p>')).toBe(false);
});
it('admits only declared widget attributes and ordinary HTML metadata', () => {
  const button = browserElement('tmt-ui-button')!;
  for (const [name, value] of [
    ['variant', 'primary'],
    ['disabled', ''],
    ['disabled', 'disabled'],
    ['aria-label', 'Save'],
    ['data-example', 'one'],
  ])
    expect(elementAttributeIssue(button, name!, value!)).toBeUndefined();
  for (const [name, value, code] of [
    ['variant', 'loud', 'invalid-ui-attribute'],
    ['disabled', 'false', 'invalid-ui-attribute'],
    ['onclick', 'save()', 'unknown-ui-attribute'],
    ['src', 'https://example.test', 'unknown-ui-attribute'],
  ])
    expect(elementAttributeIssue(button, name!, value!)).toBe(code);
});
