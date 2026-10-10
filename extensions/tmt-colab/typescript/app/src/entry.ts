/** Remote supplies the internal mount; public location carries only a display target. */
export function publicEntry(path = location.pathname): boolean {
  return (
    path === '/colab' || path === '/colab/' || /^\/(?:p|read)\/[A-Za-z0-9_-]{4,64}$/.test(path)
  );
}
export function entryMount(): URL {
  const supplied = document.querySelector<HTMLMetaElement>('meta[name="tmt-colab-mount"]')?.content;
  const url = new URL(supplied ?? location.pathname, location.origin);
  if (
    url.origin !== location.origin ||
    !/^\/r\/[a-z2-7]{16}\/x\/colab\/$/.test(url.pathname) ||
    url.search ||
    url.hash
  )
    throw new Error('Invalid Colab mount');
  return url;
}
