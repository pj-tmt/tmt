export function loadReleasePleaseCommitRules(load?: (specifier: string) => unknown): {
  CommitSplit: typeof import('../../.github/release-please/node_modules/release-please/build/src/util/commit-split.js').CommitSplit;
  CommitExclude: typeof import('../../.github/release-please/node_modules/release-please/build/src/util/commit-exclude.js').CommitExclude;
  parseConventionalCommits: typeof import('../../.github/release-please/node_modules/release-please/build/src/commit.js').parseConventionalCommits;
  DefaultChangelogNotes: typeof import('../../.github/release-please/node_modules/release-please/build/src/changelog-notes/default.js').DefaultChangelogNotes;
};
