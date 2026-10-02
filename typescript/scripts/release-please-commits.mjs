// The safety gate's single compatibility boundary for pinned release-please commit rules.
import { createRequire } from 'node:module';
import { assertReleasePleaseApi, loadPinnedReleasePlease } from './release-please-run.mjs';

const requirePinned = createRequire(
  new URL('../../.github/release-please/package.json', import.meta.url)
);

/** Fail visibly before coverage planning if an upgrade changes any internal rule interface. */
export function loadReleasePleaseCommitRules(load = requirePinned) {
  assertReleasePleaseApi(loadPinnedReleasePlease());
  try {
    const { CommitSplit } = load('release-please/build/src/util/commit-split.js');
    const { CommitExclude } = load('release-please/build/src/util/commit-exclude.js');
    const { parseConventionalCommits } = load('release-please/build/src/commit.js');
    const { DefaultChangelogNotes } = load('release-please/build/src/changelog-notes/default.js');
    if (
      typeof CommitSplit?.prototype?.split !== 'function' ||
      typeof CommitExclude?.prototype?.excludeCommits !== 'function' ||
      typeof parseConventionalCommits !== 'function' ||
      typeof DefaultChangelogNotes?.prototype?.buildNotes !== 'function'
    )
      throw new Error('Missing commit rule interface.');
    return { CommitSplit, CommitExclude, parseConventionalCommits, DefaultChangelogNotes };
  } catch (cause) {
    throw new Error(
      'Unsupported release-please commit API; re-verify notes coverage before upgrading.',
      { cause }
    );
  }
}
