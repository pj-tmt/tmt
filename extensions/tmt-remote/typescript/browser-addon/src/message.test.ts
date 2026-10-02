import { describe, expect, it, vi } from 'vite-plus/test';
import { formatMessage, visibleText, isCapture } from './message.js';
describe('selection message', () => {
  it('preserves every byte of selection, title, URL and optional note', () => {
    const capture = {
      url: 'https://example.test/?a=%0A',
      title: 'title\u202E',
      selection: '  <script>\t\r\n\u200B',
    };
    expect(isCapture(capture)).toBe(true);
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

it.each([
  'https://user@example.test/path',
  'https://:password@example.test/path',
  'https://user:password@example.test/path',
])('refuses credentialed source %s before capture storage', async (url) => {
  const capture = { url, title: 'title', selection: 'text' };
  expect(isCapture(capture)).toBe(false);
  expect(() => formatMessage(capture, '')).toThrow();
  const open = vi.fn();
  vi.stubGlobal('indexedDB', { open });
  vi.stubGlobal('chrome', { scripting: { executeScript: async () => [{ result: capture }] } });
  try {
    const { captureTab } = await import('./capture.js');
    const { saveCapture } = await import('./journal.js');
    await expect(captureTab(1)).rejects.toThrow();
    await expect(saveCapture(capture)).rejects.toThrow();
    expect(open).not.toHaveBeenCalled();
  } finally {
    vi.unstubAllGlobals();
  }
});
