import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vite-plus/test';
import { ownerOf, parseComponentMap } from '../../scripts/ci-scope.mjs';
import {
  EARLY_GATES,
  GATES,
  UNSKIPPABLE_GATE,
  REQUIRED_CONTEXTS,
  checkChannel,
  checkCommit,
  checkImmutability,
  checkMigration,
  checkMonotonic,
  checkUpgrade,
  countMigrations,
  isBreaking,
  renderGateSummary,
  runGates,
} from '../../scripts/publication-gates.mjs';

const repository = fileURLToPath(new URL('../../../', import.meta.url));
const read = (relative: string) => readFileSync(path.join(repository, relative), 'utf8');
const SHA = 'a'.repeat(40);

describe('countMigrations', () => {
  const list = (body: string) =>
    `pub const X: u8 = 1;\nconst MIGRATIONS: &[Migration] = &[\n${body}\n];\nfn f() {}`;

  it('counts the entries of a list of structs and of a list of tuples', () => {
    expect(
      countMigrations(
        list(
          'Migration { name: "one", sql: include_str!("a.sql") },\nMigration { name: "two", sql: include_str!("b.sql") },'
        )
      )
    ).toBe(2);
    expect(
      countMigrations(
        'const MIGRATIONS: &[(&str, &str)] = &[("a", include_str!("1.sql")), ("b", include_str!("2.sql")), ("c", include_str!("3.sql"))];'
      )
    ).toBe(3);
  });

  it('counts a last entry without a trailing comma, and an empty list as none', () => {
    expect(countMigrations(list('A { x: 1 },\nA { x: 2 }'))).toBe(2);
    expect(countMigrations(list(''))).toBe(0);
    expect(countMigrations(list('// nothing yet, (really), [none]'))).toBe(0);
  });

  it('ignores commas, brackets, quotes and comment markers inside strings and comments', () => {
    const source = list(
      [
        'Migration { name: "add A, B and (C) [d] {e}", sql: "x" },',
        '// a comment, with [brackets], and "a quote',
        'Migration { name: "quote \\" and , and //", sql: "y" }, /* block, ) ] } */',
        'Migration { name: "last", sql: "z" },',
      ].join('\n')
    );
    expect(countMigrations(source)).toBe(3);
  });

  it('is zero without a MIGRATIONS list and refuses one that is not closed', () => {
    expect(countMigrations('const OTHER: &[u8] = &[1, 2, 3];')).toBe(0);
    expect(() => countMigrations('const MIGRATIONS: &[u8] = &[1, 2')).toThrow('not closed');
    expect(() => countMigrations('const MIGRATIONS: &[u8] = &[1, /* open')).toThrow('unterminated');
  });

  it('agrees with an independent count of the real lists', () => {
    const cli = read('rust/crates/tmt-adapters/src/storage/migrations.rs');
    const cliRegion = cli.slice(
      cli.indexOf('const MIGRATIONS'),
      cli.indexOf('\n];', cli.indexOf('const MIGRATIONS'))
    );
    expect(countMigrations(cli)).toBe(cliRegion.match(/^ {4}Migration \{$/gm)?.length);
    expect(countMigrations(cli)).toBeGreaterThan(20);

    const office = read('extensions/tmt-office/rust/tmt-office-storage/src/schema.rs');
    const officeRegion = office.slice(
      office.indexOf('const MIGRATIONS'),
      office.indexOf('\n];', office.indexOf('const MIGRATIONS'))
    );
    expect(countMigrations(office)).toBe(officeRegion.match(/^ {4}\("/gm)?.length);
    expect(countMigrations(office)).toBeGreaterThan(0);
  });
});

describe('the component map names each component’s migration files', () => {
  const map = parseComponentMap(read('.github/components.json'));

  it('lists a tracked source file the component owns, with entries, for CLI and Office only', () => {
    const byName = Object.fromEntries(
      map.components.map((component) => [component.name, component])
    );
    expect(byName.cli.migrations).toHaveLength(1);
    expect(byName.office.migrations).toHaveLength(1);
    expect(byName.squad.migrations).toEqual([]);
    for (const component of map.components) {
      for (const file of component.migrations) {
        expect(ownerOf(file, map), file).toBe(component.name);
        expect(countMigrations(read(file)), file).toBeGreaterThan(0);
      }
    }
  });

  it('refuses migrations that are not a list of strings', () => {
    const base = JSON.parse(read('.github/components.json')) as {
      components: Record<string, { migrations?: unknown }>;
    };
    for (const migrations of [[], 'file.rs', [1]]) {
      const broken = structuredClone(base);
      broken.components.cli.migrations = migrations;
      expect(() => parseComponentMap(JSON.stringify(broken))).toThrow('components.cli.migrations');
    }
  });
});

describe('required contexts and gate order', () => {
  it('names the jobs of ci.yml that gate a merge, and the gates in the order they run', () => {
    const ci = read('.github/workflows/ci.yml');
    for (const context of REQUIRED_CONTEXTS) {
      expect(ci, context).toMatch(new RegExp(`^ {4}name: ${context}$`, 'm'));
    }
    expect(GATES).toEqual([
      'channel',
      'commit',
      'immutability',
      'monotonic',
      'migration',
      'upgrade',
    ]);
    expect(EARLY_GATES).toEqual(GATES.slice(0, 5));
    // The cheapest, tag-only gate runs first, and releasing a hold can never skip it.
    expect(GATES[0]).toBe(UNSKIPPABLE_GATE);
  });
});

describe('checkChannel', () => {
  it.each([
    ['cli', 'v5.0.0-alpha.9'],
    ['cli', 'v5.0.0-alpha.10'],
    ['office', 'tmt-office-v0.1.0-alpha.4'],
    ['squad', 'tmt-squad-v0.1.0-alpha.2'],
  ])('lets the alpha release %s %s publish automatically', (product, tag) => {
    expect(checkChannel({ product, tag }).ok).toBe(true);
  });

  it.each([
    ['cli', 'v5.0.0', 'stable'],
    ['cli', 'v5.1.3', 'stable'],
    ['squad', 'tmt-squad-v0.1.0', 'stable'],
    ['office', 'tmt-office-v1.0.0', 'stable'],
    ['cli', 'v5.0.0-beta.1', 'beta'],
    ['cli', 'v5.0.0-rc.1', 'release candidate'],
    ['squad', 'tmt-squad-v0.1.0-beta.2', 'beta'],
    ['cli', 'v5.0.0-alpha', 'alpha without a number'],
    ['cli', 'v5.0.0-alpha.1.2', 'alpha with a longer label'],
    ['cli', 'v5.0.0-alpha.x', 'alpha with a word'],
    ['cli', 'v5.0.0-alpha.9-rc.1', 'another label after the alpha'],
  ])('holds %s %s (%s) for the owner', (product, tag) => {
    const outcome = checkChannel({ product, tag });
    expect(outcome.ok).toBe(false);
    expect(outcome.reason).toContain('not an alpha release');
    expect(outcome.reason).toContain('by hand');
  });

  it('refuses a tag of another product instead of judging it', () => {
    expect(() => checkChannel({ product: 'cli', tag: 'tmt-squad-v0.1.0-alpha.2' })).toThrow(
      'is not a cli tag'
    );
  });
});

describe('checkCommit', () => {
  const run = (name: string, conclusion: string | null, at: string, status = 'completed') => ({
    name,
    status,
    conclusion,
    completed_at: at,
  });
  const green = REQUIRED_CONTEXTS.map((name) => run(name, 'success', '2026-09-30T01:00:00Z'));
  const check = (input: Partial<Parameters<typeof checkCommit>[0]>) =>
    checkCommit({ sha: SHA, onMain: true, pullRequest: { number: 7 }, checkRuns: green, ...input });

  it('passes a commit on main whose pull request passed every required context', () => {
    expect(check({}).ok).toBe(true);
  });

  it('holds a commit that is not on main or was not produced by a pull request', () => {
    expect(check({ onMain: false })).toEqual({
      ok: false,
      reason: 'commit aaaaaaaa is not on main',
    });
    expect(check({ pullRequest: null }).reason).toContain(
      'no merged pull request produced commit aaaaaaaa'
    );
  });

  it('holds a required context that did not complete or did not succeed', () => {
    for (const context of REQUIRED_CONTEXTS) {
      const without = green.filter((entry) => entry.name !== context);
      expect(check({ checkRuns: without }).reason).toBe(
        `required check "${context}" did not complete on #7`
      );
      expect(
        check({ checkRuns: [...without, run(context, 'failure', '2026-09-30T01:00:00Z')] }).reason
      ).toBe(`required check "${context}" failure on #7`);
      expect(
        check({
          checkRuns: [...without, run(context, null, '2026-09-30T01:00:00Z', 'in_progress')],
        }).ok
      ).toBe(false);
    }
  });

  it('counts the latest run of a context, so a re-run that passed replaces a failure', () => {
    const failedThenPassed = [
      ...green.filter((entry) => entry.name !== 'Unit tests'),
      run('Unit tests', 'failure', '2026-09-30T00:00:00Z'),
      run('Unit tests', 'success', '2026-09-30T02:00:00Z'),
    ];
    expect(check({ checkRuns: failedThenPassed }).ok).toBe(true);
    const passedThenFailed = [
      ...green.filter((entry) => entry.name !== 'Unit tests'),
      run('Unit tests', 'success', '2026-09-30T00:00:00Z'),
      run('Unit tests', 'failure', '2026-09-30T02:00:00Z'),
    ];
    expect(check({ checkRuns: passedThenFailed }).ok).toBe(false);
  });

  it('ignores advisory checks that failed', () => {
    const advisory = run('Native Office browser (2/8)', 'failure', '2026-09-30T02:00:00Z');
    expect(check({ checkRuns: [...green, advisory] }).ok).toBe(true);
  });
});

describe('checkImmutability', () => {
  const release = (tag: string, published: string, immutable: boolean, draft = false) => ({
    tag_name: tag,
    draft,
    published_at: published,
    immutable,
  });

  it('passes when the newest published release of the repository is immutable', () => {
    expect(
      checkImmutability({
        releases: [
          release('v5.0.0-alpha.7', '2026-09-29T12:00:00Z', false),
          release('v5.0.0-alpha.8', '2026-09-29T15:00:00Z', true),
          release('tmt-office-v0.1.0-alpha.4', '2026-09-30T00:00:00Z', false, true),
        ],
      })
    ).toEqual({ ok: true, reason: 'v5.0.0-alpha.8 is immutable' });
  });

  it('holds when the newest published release is not immutable, whatever the older ones are', () => {
    const result = checkImmutability({
      releases: [
        release('v5.0.0-alpha.7', '2026-09-29T12:00:00Z', true),
        release('tmt-squad-v0.1.0-alpha.2', '2026-09-29T16:00:00Z', false),
      ],
    });
    expect(result.ok).toBe(false);
    expect(result.reason).toContain('tmt-squad-v0.1.0-alpha.2 is not immutable');
  });

  it('holds when nothing is published, because nothing shows the setting', () => {
    expect(checkImmutability({ releases: [] }).ok).toBe(false);
    expect(checkImmutability({ releases: [release('v5.0.0-alpha.9', '', true, true)] }).ok).toBe(
      false
    );
  });
});

describe('checkMonotonic', () => {
  const releases = [
    { tag_name: 'v5.0.0-alpha.8', draft: false },
    { tag_name: 'v5.0.0-alpha.10', draft: true },
    { tag_name: 'tmt-office-v0.1.0-alpha.9', draft: false },
  ];

  it('passes a release newer than everything published of its product', () => {
    expect(checkMonotonic({ releases, product: 'cli', tag: 'v5.0.0-alpha.9' }).ok).toBe(true);
    expect(checkMonotonic({ releases, product: 'cli', tag: 'v5.0.0-alpha.10' }).ok).toBe(true);
    expect(checkMonotonic({ releases, product: 'squad', tag: 'tmt-squad-v0.1.0-alpha.1' }).ok).toBe(
      true
    );
  });

  it('holds a release that is equal to or older than one already published', () => {
    for (const tag of ['v5.0.0-alpha.8', 'v5.0.0-alpha.7', 'v4.9.9']) {
      const result = checkMonotonic({ releases, product: 'cli', tag });
      expect(result.ok, tag).toBe(false);
      expect(result.reason).toContain('v5.0.0-alpha.8 is already published');
    }
  });
});

describe('checkMigration', () => {
  const files = ['rust/crates/tmt-adapters/src/storage/migrations.rs'];
  const previous = { tag: 'v5.0.0-alpha.8', counts: { [files[0]]: 24 } };
  const commit = (subject: string, body = '') => ({ sha: 'b'.repeat(40), subject, body });
  const check = (counts: Record<string, number>, commits = [commit('fix: a bug')], alpha = false) =>
    checkMigration({ files, counts, previous, commits, alpha });

  it('passes a release with the same migrations and no breaking commit', () => {
    expect(check({ [files[0]]: 24 }).ok).toBe(true);
  });

  it('holds a release with more entries than the last published one', () => {
    const result = check({ [files[0]]: 25 });
    expect(result.ok).toBe(false);
    expect(result.reason).toBe(`${files[0]} has 25 migrations, 24 in v5.0.0-alpha.8`);
  });

  it('holds a file the last published release did not have', () => {
    const result = checkMigration({
      files,
      counts: { [files[0]]: 1 },
      previous: { tag: 'v5.0.0-alpha.8', counts: {} },
      commits: [],
    });
    expect(result.reason).toContain('1 migrations, 0 in v5.0.0-alpha.8');
  });

  it('publishes an alpha with more entries and says how many, outside the alpha channel it holds', () => {
    const alpha = check({ [files[0]]: 27 }, [commit('feat: a table')], true);
    expect(alpha.ok).toBe(true);
    expect(alpha.reason).toContain(`${files[0]} has 27 migrations, 24 in v5.0.0-alpha.8`);
    expect(alpha.reason).toContain('an alpha publishes its migrations');
    expect(check({ [files[0]]: 27 }, [commit('feat: a table')], false).ok).toBe(false);
    // The same for a file the last published release did not have.
    expect(
      checkMigration({
        files,
        counts: { [files[0]]: 1 },
        previous: { tag: 'v5.0.0-alpha.8', counts: {} },
        commits: [],
        alpha: true,
      }).ok
    ).toBe(true);
  });

  it('still holds an alpha that carries a breaking commit, with or without a new migration', () => {
    for (const count of [24, 25]) {
      const result = check({ [files[0]]: count }, [commit('feat(api)!: drop the flag')], true);
      expect(result.ok).toBe(false);
      expect(result.reason).toContain('is a breaking change: feat(api)!: drop the flag');
    }
    expect(
      check(
        { [files[0]]: 25 },
        [commit('feat: rename', 'Why.\n\nBREAKING CHANGE: the flag is gone')],
        true
      ).ok
    ).toBe(false);
  });

  it('holds a breaking commit, by its marker or its footer', () => {
    expect(check({ [files[0]]: 24 }, [commit('feat(api)!: drop the flag')]).reason).toContain(
      'is a breaking change: feat(api)!: drop the flag'
    );
    expect(
      check({ [files[0]]: 24 }, [
        commit('feat: rename', 'Why.\n\nBREAKING CHANGE: the flag is gone'),
      ]).ok
    ).toBe(false);
  });

  it('passes the first release of a product, and a component without migration files', () => {
    expect(checkMigration({ files, counts: {}, previous: null, commits: [] }).ok).toBe(true);
    expect(
      checkMigration({ files: [], counts: {}, previous, commits: [commit('chore: tidy')] }).ok
    ).toBe(true);
    expect(
      checkMigration({ files: [], counts: {}, previous, commits: [commit('fix!: a break')] }).ok
    ).toBe(false);
  });
});

describe('isBreaking', () => {
  it('recognizes a conventional-commit marker and a footer, and nothing else', () => {
    for (const subject of ['feat!: x', 'fix(scope)!: x', 'refactor(a-b)!: x']) {
      expect(isBreaking({ subject }), subject).toBe(true);
    }
    expect(isBreaking({ subject: 'chore: x', body: 'a\nBREAKING CHANGE: y' })).toBe(true);
    expect(isBreaking({ subject: 'chore: x', body: 'a\nBREAKING-CHANGE: y' })).toBe(true);
    for (const subject of [
      'feat: bang! in text',
      'fix: a!: b',
      'Revert "feat!: x"',
      'feat(a): x',
    ]) {
      expect(isBreaking({ subject }), subject).toBe(false);
    }
    expect(isBreaking({ subject: 'feat: x', body: 'not a BREAKING CHANGE: footer' })).toBe(false);
  });
});

describe('checkUpgrade', () => {
  it('passes a proof, and a first release with nothing to upgrade from', () => {
    expect(checkUpgrade({ result: 'success', outcome: 'proved' }).ok).toBe(true);
    expect(checkUpgrade({ result: 'success', outcome: 'nothing' }).reason).toContain(
      'nothing to upgrade from'
    );
  });

  it('holds a commit that predates the proof with the reason verbatim', () => {
    const reason =
      "This release's commit has no release-upgrade.mjs: it predates the automated upgrade proof.";
    expect(checkUpgrade({ result: 'success', outcome: 'predates', reason })).toEqual({
      ok: false,
      reason,
    });
  });

  it('holds a proof that failed, was cancelled or was skipped, with the run', () => {
    for (const result of ['failure', 'cancelled', 'skipped']) {
      const held = checkUpgrade({ result, outcome: '', url: 'https://example.test/run' });
      expect(held, result).toEqual({
        ok: false,
        reason: `the upgrade proof ${result}: https://example.test/run`,
      });
    }
    expect(checkUpgrade({ result: 'success', outcome: '' }).ok).toBe(false);
  });

  it('puts the cause the failed hosts name in front of the run, and still cites the run without one', () => {
    const cause =
      'Packed command failed (exited 1, expected 0): tmt extension install squad: unrecognized subcommand squad';
    expect(
      checkUpgrade({
        result: 'failure',
        outcome: 'proved',
        reason: cause,
        url: 'https://example.test/run',
      }).reason
    ).toBe(`the upgrade proof failure: ${cause} (https://example.test/run)`);
    expect(checkUpgrade({ result: 'failure', outcome: 'proved', reason: cause }).reason).toBe(
      `the upgrade proof failure: ${cause}`
    );
    expect(checkUpgrade({ result: 'failure', outcome: 'proved' }).reason).toBe(
      'the upgrade proof failure'
    );
  });
});

describe('runGates', () => {
  const ok = { ok: true, reason: '' };
  const make = (failing = '') => {
    const called: string[] = [];
    const checks = Object.fromEntries(
      EARLY_GATES.map((gate) => [
        gate,
        () => {
          called.push(gate);
          return gate === failing ? { ok: false, reason: `${gate} failed` } : ok;
        },
      ])
    );
    return { called, checks };
  };

  it('runs every gate in order when they pass', () => {
    const { called, checks } = make();
    const { held, results } = runGates({ order: EARLY_GATES, checks });
    expect(held).toBeNull();
    expect(called).toEqual(EARLY_GATES);
    expect(results.map((result) => result.gate)).toEqual(EARLY_GATES);
  });

  it('stops at the first failure and gathers no evidence for the gates after it', () => {
    const { called, checks } = make('monotonic');
    const { held } = runGates({ order: EARLY_GATES, checks });
    expect(held).toEqual({ gate: 'monotonic', reason: 'monotonic failed' });
    expect(called).toEqual(['channel', 'commit', 'immutability', 'monotonic']);
  });

  it('skips exactly the named gate, and still stops at a different failure', () => {
    const { called, checks } = make('commit');
    const released = runGates({ order: EARLY_GATES, checks, skip: 'commit' });
    expect(released.held).toBeNull();
    expect(called).toEqual(['channel', 'immutability', 'monotonic', 'migration']);
    expect(released.results[1]).toMatchObject({ gate: 'commit', skipped: true });

    const other = make('migration');
    expect(runGates({ order: EARLY_GATES, checks: other.checks, skip: 'commit' }).held?.gate).toBe(
      'migration'
    );
  });
});

describe('renderGateSummary', () => {
  it('lists each gate and says what a held draft carries, or that the next job publishes', () => {
    const results = [
      { gate: 'commit', ok: true, reason: '#7 passed' },
      { gate: 'monotonic', ok: false, reason: 'v5.0.0-alpha.8 is already published' },
    ];
    const held = renderGateSummary({
      tag: 'v5.0.0-alpha.7',
      results,
      held: { gate: 'monotonic', reason: 'v5.0.0-alpha.8 is already published' },
    });
    expect(held).toContain('### Publication gates for `v5.0.0-alpha.7`');
    expect(held).toContain('- passed `commit`: #7 passed');
    expect(held).toContain('- FAILED `monotonic`');
    expect(held).toContain('**Held** at `monotonic`');
    expect(held).toContain('publication-held.json');
    const passed = renderGateSummary({ tag: 'v5.0.0-alpha.9', results: [results[0]], held: null });
    expect(passed).toContain('Every gate passed. The next job publishes the release.');
  });
});
