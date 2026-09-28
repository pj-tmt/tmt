/** Syntax only: no fetch, navigation, credentials or implicit base URL. */
export function externalLink(value: unknown): URL {
  if (
    typeof value !== 'string' ||
    new TextEncoder().encode(value).length > 2048 ||
    // oxlint-disable-next-line no-control-regex -- URL admission intentionally rejects raw C0/C1 controls.
    /[\s\u0000-\u0020\u007f-\u009f\\\u202a-\u202e\u2066-\u2069]/u.test(value) ||
    !/^https?:\/\//i.test(value)
  )
    throw new Error('Enter an absolute HTTP(S) URL without credentials or control characters.');
  const authority = value.split('://')[1]?.split(/[/?#]/)[0];
  if (!authority || authority.includes('@'))
    throw new Error('Web links require a host without credentials.');
  const url = new URL(value);
  if (!url.hostname || url.username || url.password || !['http:', 'https:'].includes(url.protocol))
    throw new Error('Invalid web destination.');
  return url;
}
