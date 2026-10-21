import { expect, it } from 'vite-plus/test';
import { text } from '../src/strings.js';

it('says the same plain sentence about a page navigating its own frame everywhere', () => {
  expect(text.frameLimit).toContain('Colab cannot fully prevent that.');
  expect(text.warning).toContain(text.frameLimit);
  // The contract forbids claiming complete prevention; the words stay plain.
  for (const sentence of [text.frameLimit, text.warning]) {
    expect(sentence).not.toMatch(/exfiltration|navigate/i);
    expect(sentence).not.toMatch(/prevent(s|ed|ion) (all|complete)/i);
  }
});
