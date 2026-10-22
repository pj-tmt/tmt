/** The pure reading of an app build's name (#4282): the content-hashed entry module a page
 * references. The app and `vite.config.ts` share it, so the stamp the module is built with and the
 * entry a later read finds are named by one rule. */

const ENTRY_SCRIPTS = /<script\b[^>]*\btype="module"[^>]*\bsrc="([^"]+)"/gi;
const READ_LIMIT = 64 * 1024;

/** The app build is the content-hashed entry module a page references; a page may also
 * carry tooling modules (the dev server's client), so the hashed bundle wins. */
export function pick(paths: string[]): string | null {
  return paths.find((path) => stamp(path)) ?? paths[0] ?? null;
}

export function entryOf(html: string, base: URL): string | null {
  const paths: string[] = [];
  for (const match of html.slice(0, READ_LIMIT).matchAll(ENTRY_SCRIPTS)) {
    try {
      paths.push(new URL(match[1], base).pathname);
    } catch {
      // A malformed reference is not an entry.
    }
  }
  return pick(paths);
}

/** `/assets/index-AbC123xY.js` is build `index` at hash `AbC123xY`; unhashed entries
 * (the guidance page's `recovery.js`, the reader's `reader.js`) are not app builds.
 * This relies on Vite's 8-character content hash: if the build's hash length changes the
 * notice goes silent, which `test/build-stamp.test.ts` pins. */
function stamp(path: string): { name: string; hash: string } | null {
  const match = /([^/]+)-([A-Za-z0-9_-]{8})\.js$/.exec(path);
  return match ? { name: match[1], hash: match[2] } : null;
}

/** Whether `path` names an app build: a content-hashed entry, as `stamp` reads it. */
export function isBuild(path: string): boolean {
  return stamp(path) !== null;
}

/** A real upgrade: the same bundle name at a different hash. A different kind of
 * page (such as pairing guidance served to an unpaired browser) says nothing. */
export function isNewerBuild(loaded: string, served: string): boolean {
  const have = stamp(loaded);
  const now = stamp(served);
  return !!have && !!now && have.name === now.name && have.hash !== now.hash;
}
