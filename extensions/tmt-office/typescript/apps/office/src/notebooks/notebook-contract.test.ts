import { expect, it } from 'vitest';
import { decodeNotebook, NOTEBOOK_READ_BYTES } from './notebook-contract.js';

const notebook = { identityId: '11111111-1111-4111-8111-111111111111', name: 'Alice', content: '' };

it('retains exact valid UTF-8, including markup, whitespace, NUL and supplementary characters', () => {
  const content = '\ufeff# Notes\r\n<script>unsafe()</script>\n\u0000🤖  ';
  expect(decodeNotebook({ ...notebook, content })).toEqual({ ...notebook, content });
  expect(decodeNotebook(notebook)).toEqual(notebook);
  expect(
    decodeNotebook({ ...notebook, content: 'a'.repeat(NOTEBOOK_READ_BYTES) }).content.length
  ).toBe(NOTEBOOK_READ_BYTES);
});

it('rejects unsafe scope, unknown metadata, malformed text and bytes beyond the viewer ceiling', () => {
  for (const value of [
    { ...notebook, identityId: '../notes' },
    { ...notebook, file: '/private/notes.md' },
    { ...notebook, content: '\ud800' },
    { ...notebook, content: 'a'.repeat(NOTEBOOK_READ_BYTES + 1) },
    { ...notebook, content: 'é'.repeat(NOTEBOOK_READ_BYTES / 2 + 1) },
  ])
    expect(() => decodeNotebook(value)).toThrow();
});
