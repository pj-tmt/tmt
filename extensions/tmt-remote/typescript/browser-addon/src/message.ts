export interface Capture {
  selection: string;
  url: string;
  title: string;
}
export function isSourceUrl(value: string): boolean {
  try {
    const url = new URL(value);
    return (
      /^https?:\/\//i.test(value) &&
      !/[\r\n\t]/.test(value) &&
      ['http:', 'https:'].includes(url.protocol) &&
      !url.username &&
      !url.password
    );
  } catch {
    return false;
  }
}
export function assertMessageSource(message: string): void {
  const firstLine = message.split('\n', 1)[0];
  if (!firstLine.startsWith('[browser] ') || !isSourceUrl(firstLine.slice(10)))
    throw new Error('Source URL must be HTTP(S) without credentials.');
}
export function isCapture(value: unknown): value is Capture {
  if (!value || typeof value !== 'object') return false;
  const v = value as Record<string, unknown>;
  return (
    ['selection', 'url', 'title'].every((k) => typeof v[k] === 'string') &&
    isSourceUrl(v.url as string) &&
    new TextEncoder().encode([v.selection, v.url, v.title].join('')).length <= 65536
  );
}
export function formatMessage(capture: Capture, note: string): string {
  const message =
    `[browser] ${capture.url}\nTitle: ${capture.title}\n\nSelection:\n${capture.selection}` +
    (note ? `\n\nNote:\n${note}` : '');
  if (!isCapture(capture) || !capture.selection || new TextEncoder().encode(message).length > 65536)
    throw new Error('Select some text; the complete message must fit within 64 KiB.');
  return message;
}
export function visibleText(text: string): string {
  return Array.from(text, (character) => {
    const code = character.codePointAt(0)!;
    if (character === '\\') return '\\\\';
    if (character === '\n') return '\\n\n';
    if (character === '\t') return '\\t';
    // Show controls, format/bidi characters, unusual spaces and lone surrogates.
    if (/[\p{Cc}\p{Cf}\p{Z}\p{Cs}\p{Mn}]/u.test(character) && character !== ' ')
      return `\\u{${code.toString(16).toUpperCase().padStart(4, '0')}}`;
    return character;
  }).join('');
}
