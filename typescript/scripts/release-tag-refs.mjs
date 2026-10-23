// Reconcile captured publication metadata with local immutable tag refs, once.
// Both publication gates and Project evidence use this owner; callers own git IO.
const TAG = /^[A-Za-z0-9][A-Za-z0-9._-]*$/;
const COMMIT = /^[a-f0-9]{40} commit$/;

export function reconcilePublishedTags(tags, git) {
  const names = [...new Set(tags)];
  if (!names.length) return;
  const invalid = names.filter(
    (tag) =>
      typeof tag !== 'string' || !TAG.test(tag) || tag.includes('..') || /\.$|\.lock$/.test(tag)
  );
  if (invalid.length)
    throw new Error(
      `Git evidence failed: unsupported published tag name (not fetched): ${invalid.join(', ')}`
    );
  const read = () => {
    const rows = git(
      ['cat-file', '--batch-check=%(objectname) %(objecttype)'],
      names.map((tag) => `refs/tags/${tag}^{commit}\n`).join('')
    )
      .trim()
      .split('\n');
    if (rows.length !== names.length)
      throw new Error('Git evidence failed: unexpected tag check output.');
    return rows;
  };
  const before = read();
  let missing = names.filter((_, index) => !COMMIT.test(before[index]));
  if (!missing.length) return;
  const refusal = (detail = '') =>
    new Error(
      `Git evidence failed: published tag(s) do not resolve to a commit: ${missing.join(', ')}${detail}`
    );
  try {
    // No force, wildcard or automatic tag discovery. stdin keeps argv bounded
    // as the catalogue grows; only the captured missing exact refs are fetched.
    git(
      [
        'fetch',
        '--no-tags',
        '--no-recurse-submodules',
        '--no-write-fetch-head',
        '--quiet',
        '--stdin',
        'origin',
      ],
      missing.map((tag) => `refs/tags/${tag}:refs/tags/${tag}\n`).join('')
    );
  } catch (error) {
    throw refusal(` (${error.message})`);
  }
  const after = read();
  const changed = names.filter(
    (_, index) => COMMIT.test(before[index]) && before[index] !== after[index]
  );
  if (changed.length)
    throw new Error(
      `Git evidence failed: existing published tag commit changed: ${changed.join(', ')}`
    );
  missing = names.filter((_, index) => !COMMIT.test(after[index]));
  if (missing.length) throw refusal();
}
