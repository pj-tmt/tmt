import { describe, expect, it } from 'vitest';
import { formatMessage, visibleText } from './message.js';
describe('selection message', () => {
  it('preserves every byte of selection, title, URL and optional note', () => {
    const capture = {
      url: 'https://example.test/?a=%0A',
      title: 'title\u202E',
      selection: '  <script>\t\r\n\u200B',
    };
    expect(formatMessage(capture, ' note\u0000')).toBe(
      '[browser] https://example.test/?a=%0A\nTitle: title\u202E\n\nSelection:\n  <script>\t\r\n\u200B\n\nNote:\n note\u0000',
    );
    expect(formatMessage(capture, '')).not.toContain('Note:');
  });
  it('shows escapes unambiguously without altering submitted text', () => {
    expect(visibleText('\\n\n\t\r\u0000\u202E\u200B\u00A0\u2028\ud800\u034F')).toBe(
      '\\\\n\\n\n\\t\\u{000D}\\u{0000}\\u{202E}\\u{200B}\\u{00A0}\\u{2028}\\u{D800}\\u{034F}',
    );
  });
  it('rejects empty and oversized complete messages rather than truncating', () => {
    expect(() => formatMessage({ url: 'https://x', title: '', selection: '' }, '')).toThrow();
    expect(() =>
      formatMessage({ url: 'https://x', title: '', selection: 'é'.repeat(32768) }, ''),
    ).toThrow();
  });
});
