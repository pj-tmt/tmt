import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync, symlinkSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vite-plus/test';
import * as tar from 'tar';

const proof = await import(new URL('../../scripts/workspace-proof.mjs', import.meta.url).href);
const transport = await import(
  new URL('../../scripts/run-workspace-proof.mjs', import.meta.url).href
);
const root = fileURLToPath(new URL('../../../', import.meta.url));
const clone = <T>(value: T): T => JSON.parse(JSON.stringify(value));
const names = ['tmt-cli', 'tmt-office-command', 'tmt-office-model', 'tmt-remote'];
function fixture() {
  const packages = [...names, ...proof.EXCLUSIONS].map((name: string) => {
    const cwd = `/source/rust/crates/${name}`;
    return {
      id: name,
      name,
      manifestPath: `${cwd}/Cargo.toml`,
      targets: [
        {
          name: name === 'tmt-cli' ? 'architecture' : name.replaceAll('-', '_'),
          src_path: `${cwd}/${name === 'tmt-cli' ? 'tests/architecture.rs' : 'src/lib.rs'}`,
          kind: [name === 'tmt-cli' ? 'test' : 'lib'],
          crate_types: [name === 'tmt-cli' ? 'bin' : 'lib'],
          test: true,
          doctest: name !== 'tmt-cli',
          edition: '2024',
        },
      ],
    };
  });
  const artifacts = packages
    .filter((pkg: { name: string }) => names.includes(pkg.name))
    .flatMap((pkg: { id: string; targets: unknown[] }) =>
      [true, false].map((test) => ({
        reason: 'compiler-artifact',
        package_id: pkg.id,
        target: pkg.targets[0],
        profile: { test, opt_level: '0' },
        features: ['default'],
        executable: test ? `/source/rust/target/debug/deps/${pkg.id}` : null,
        filenames: [
          test
            ? `/source/rust/target/debug/deps/${pkg.id}`
            : `/source/rust/target/debug/deps/${pkg.id}-support`,
        ],
      }))
    );
  const manifests = Object.fromEntries(
    packages.map((pkg: { name: string }) => [pkg.name, { package: {} }])
  );
  return { packages, artifacts, manifests };
}
const listed = 'alpha: test\nignored: test\n\n2 tests, 0 benchmarks\n';
const ignored = 'ignored: test\n1 test, 0 benchmarks\n';
const executed =
  'running 2 tests\ntest alpha ... ok\ntest ignored ... ignored, deliberate fixture\n\ntest result: ok. 1 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.01s\n';

describe('N=1 Cargo proof admission (synthetic formats, not Linux equivalence)', () => {
  it('owns exactly the retained workspace selection and one no-run compilation', () => {
    expect(proof.cargoArgs('test', ['--no-run', '--message-format=json'])).toEqual([
      '+1.97.0',
      'test',
      '--locked',
      '--workspace',
      '--exclude',
      'tmt-office',
      '--exclude',
      'tmt-office-storage',
      '--exclude',
      'tmt-office-pairing',
      '--exclude',
      'tmt-office-service',
      '--no-run',
      '--message-format=json',
    ]);
    expect(() => proof.cargoArgs('metadata')).toThrow();
  });
  it('requires actual compiler-artifacts, a final successful compilation and distinct units', () => {
    const f = fixture();
    const bytes =
      f.artifacts.map((record) => JSON.stringify(record)).join('\n') +
      '\n{"reason":"build-finished","success":true}\n';
    expect(proof.cargoOutput(bytes).records).toHaveLength(f.artifacts.length + 1);
    for (const bad of [
      bytes.replace('"success":true', '"success":false'),
      bytes.replace('{"reason":"build-finished","success":true}', ''),
      `${bytes}{"reason":"build-finished","success":true}`,
      `${bytes}unclassified\n`,
      '{not-json}\n',
    ])
      expect(() => proof.cargoOutput(bad)).toThrow();
    expect(() =>
      proof.admitInventory(
        [
          ...f.artifacts,
          { ...f.artifacts[0], target: { ...f.packages[0].targets[0], name: 'extra' } },
        ],
        f.packages,
        f.manifests
      )
    ).toThrow('Unknown workspace artifact');
    const inv = proof.admitInventory(f.artifacts, f.packages, f.manifests);
    expect(inv.harnesses).toHaveLength(4);
    expect(inv.docs).toHaveLength(3);
    expect(proof.assignment(inv.harnesses)[0].ids).toEqual(
      inv.harnesses.map((h: { id: string }) => h.id).sort()
    );
    expect(() => proof.assignment([...inv.harnesses, inv.harnesses[0]])).toThrow('Duplicate');
    expect(() =>
      proof.admitInventory([...f.artifacts, f.artifacts[0]], f.packages, f.manifests)
    ).toThrow('Duplicate');
    expect(() => proof.admitInventory(f.artifacts.slice(1), f.packages, f.manifests)).toThrow(
      'Missing'
    );
    const unsupported = clone(f);
    Object.assign(unsupported.manifests['tmt-remote'], { lib: { harness: false } });
    expect(() =>
      proof.admitInventory(unsupported.artifacts, unsupported.packages, unsupported.manifests)
    ).toThrow('harness=false');
  });
  it('keeps compile-only examples out of execution and refuses unknown target eligibility', () => {
    const f = fixture();
    const example = {
      name: 'support',
      src_path: '/source/rust/crates/tmt-remote/examples/support.rs',
      kind: ['example'],
      crate_types: ['bin'],
      test: false,
      doctest: false,
      edition: '2024',
    };
    f.packages[3].targets.push(example);
    f.artifacts.push({
      ...f.artifacts[7],
      target: example,
      executable: '/source/rust/target/debug/examples/support',
      filenames: ['/source/rust/target/debug/examples/support'],
    });
    const inv = proof.admitInventory(f.artifacts, f.packages, f.manifests);
    expect(inv.harnesses).toHaveLength(4);
    expect(
      inv.support.find((s: { target: { name: string } }) => s.target.name === 'support').disposition
    ).toBe('compile-support');
    const missing = f.artifacts.slice(0, -1);
    expect(() => proof.admitInventory(missing, f.packages, f.manifests)).toThrow(
      'Missing compile/support'
    );
    const unknown = clone(f);
    unknown.packages[3].targets[1].kind = ['future-target'];
    unknown.artifacts[unknown.artifacts.length - 1].target = unknown.packages[3].targets[1];
    expect(() =>
      proof.admitInventory(unknown.artifacts, unknown.packages, unknown.manifests)
    ).toThrow('Unsupported');
  });
  it('proves exact ignored, zero-test and complete execution disposition', () => {
    const list = proof.enumeration(listed, ignored);
    expect(proof.parseExecution(executed, list)).toEqual([
      ['alpha', 'ok'],
      ['ignored', 'ignored'],
    ]);
    expect(proof.enumeration('0 tests, 0 benchmarks', '0 tests, 0 benchmarks')).toEqual({
      names: [],
      ignored: [],
    });
    expect(
      proof.parseExecution(
        'running 0 tests\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s',
        { names: [], ignored: [] }
      )
    ).toEqual([]);
    for (const bad of [
      '0 tests, 0 benchmarks\n0 tests, 0 benchmarks',
      listed.replace('2 tests', '3 tests'),
      `${listed}extra: test`,
      listed.replace('0 benchmarks', '1 benchmark'),
      'alpha: test\nalpha: test\n2 tests, 0 benchmarks',
    ])
      expect(() => proof.parseList(bad)).toThrow();
    expect(() => proof.enumeration(listed, 'extra: test\n1 test, 0 benchmarks')).toThrow('subset');
    for (const bad of [
      executed.replace('test alpha ... ok\n', ''),
      executed.replace('test alpha ... ok', 'test extra ... ok'),
      executed.replace('test alpha ... ok', 'test alpha ... ok\ntest alpha ... ok'),
      executed.replace('0 filtered', '1 filtered'),
      executed.replace('test ignored ... ignored, deliberate fixture', 'test ignored ... ok'),
      executed.replace('test result: ok.', 'test result: FAILED.'),
      `${executed}unclassified child output\n`,
    ])
      expect(() => proof.parseExecution(bad, list)).toThrow();
  });
  it('compares shared library features, never repairing a different doc graph', () => {
    expect(() => proof.compareDocFeatures([['lib', ['a']]], [['lib', ['a']]])).not.toThrow();
    expect(() => proof.compareDocFeatures([['lib', ['a']]], [['lib', ['a', 'b']]])).toThrow(
      'feature'
    );
    expect(() => proof.compareDocFeatures([['lib', ['a']]], [['other', ['a']]])).toThrow('absent');
    expect(() => proof.compareDocFeatures([['lib', ['a']]], [])).toThrow('Missing');
  });
  it('attributes multiple Rust 2024 doctest blocks and explicit zero obligations', () => {
    const inv = {
      rustRoot: '/source/rust',
      harnesses: [],
      docs: [
        { package: 'lib', target: 'lib', cwd: '/source/rust/crates/lib' },
        { package: 'zero', target: 'zero', cwd: '/source/rust/crates/zero' },
      ],
      docLists: {
        'crates/lib/src/lib.rs - first (line 1)': { ignored: false },
        'crates/lib/src/lib.rs - second (line 9)': { ignored: false },
      },
    };
    const finish = '{"reason":"build-finished","success":true}\n';
    const stdout = `${finish}crates/lib/src/lib.rs - first (line 1): test\n1 test, 0 benchmarks\ncrates/lib/src/lib.rs - second (line 9): test\n1 test, 0 benchmarks\n0 tests, 0 benchmarks\n`;
    const stderr = '   Doc-tests lib\n   Doc-tests zero\n';
    expect(proof.cargoCoverage(stdout, stderr, inv, 'list').docs).toEqual([
      { package: 'lib', values: Object.keys(inv.docLists), ignored: false },
      { package: 'zero', values: [], ignored: false },
    ]);
    expect(() =>
      proof.cargoCoverage(stdout, stderr.replace('   Doc-tests zero\n', ''), inv, 'list')
    ).toThrow('target');
    expect(() =>
      proof.cargoCoverage(stdout.replace('crates/lib', 'crates/outside'), stderr, inv, 'list')
    ).toThrow('owner');
    expect(() => proof.cargoCoverage(stdout + 'ambiguous\n', stderr, inv, 'list')).toThrow(
      'Truncated'
    );
  });
  it('requires every original selected job, identity, payload and binary/doc union', () => {
    const good = ['baseline', 'producer', 'consumer', 'doctest'].map((role) => ({
      role,
      complete: true,
      runtimeRootsRemoved: true,
      identity: { source: 'head', attempt: '1' },
      commands: [{ status: 0, signal: null, cleanup: true }],
      inventory: { harnesses: [{ id: 'binary' }] },
      inventoryHash: 'a'.repeat(64),
      bundleHash: 'b'.repeat(64),
      assignment: [{ index: 0, ids: ['binary'] }],
      execution: [{ id: 'binary', values: [['case', 'ok']] }],
      docs: ['doc'],
      libraryFeatures: [['lib', ['a']]],
      controls: ['cargo', 'child', 'library', 'source'].map((kind) => ({
        kind,
        positive: true,
        missing: true,
        changedHash: true,
        restored: true,
        inputHash: 'a'.repeat(64),
      })),
      closureVerified: true,
      cleanupVerified: true,
    }));
    const results = Object.fromEntries(good.map((report) => [report.role, 'success']));
    expect(proof.aggregate(good, results).complete).toBe(true);
    for (const status of ['failure', 'cancelled', 'skipped', 'missing'])
      expect(() => proof.aggregate(good, { ...results, consumer: status })).toThrow('proof job');
    for (const reports of [good.slice(1), [...good, good[0]]])
      expect(() => proof.aggregate(reports, results)).toThrow('report');
    const mutate = (role: string, field: string, value: unknown) => {
      const changed = clone(good);
      Object.assign(
        changed.find((report) => report.role === role)!,
        { [field]: value }
      );
      return changed;
    };
    for (const [role, field, value] of [
      ['consumer', 'identity', { source: 'other', attempt: '1' }],
      ['consumer', 'identity', { source: 'head', attempt: '2' }],
      ['consumer', 'inventoryHash', 'wrong'],
      ['consumer', 'bundleHash', 'wrong'],
      ['consumer', 'assignment', []],
      ['consumer', 'execution', ['binary/extra']],
      ['consumer', 'cleanupVerified', false],
      ['doctest', 'docs', []],
      ['doctest', 'libraryFeatures', [['lib', ['wrong']]]],
      ['producer', 'complete', false],
      ['baseline', 'commands', [{ status: 1, signal: null, cleanup: true }]],
    ] as const)
      expect(() => proof.aggregate(mutate(role, field, value), results)).toThrow();
  });
});

describe('bounded frozen transport', () => {
  it('binds original process records and both streams; missing/extra or tampered evidence is red', () => {
    const temp = mkdtempSync(path.join(tmpdir(), 'tmt-workspace-proof-records-'));
    try {
      const record = {
        id: '000-test',
        status: 0,
        signal: null,
        cleanup: true,
        stdout: {},
        stderr: {},
      };
      for (const stream of ['stdout', 'stderr'] as const) {
        writeFileSync(path.join(temp, `000-test.${stream}`), stream);
        record[stream] = transport.hashFile(path.join(temp, `000-test.${stream}`));
      }
      const report = { role: 'baseline', commands: [record] };
      for (const name of ['baseline.json', 'source-files.json', 'metadata.raw.json'])
        writeFileSync(path.join(temp, name), '{}');
      writeFileSync(path.join(temp, '000-test.process.json'), JSON.stringify(record));
      transport.verifyReportFiles(temp, report);
      writeFileSync(path.join(temp, '000-test.stdout'), 'tampered');
      expect(() => transport.verifyReportFiles(temp, report)).toThrow('stream hash');
      writeFileSync(path.join(temp, '000-test.stdout'), 'stdout');
      writeFileSync(path.join(temp, 'extra'), 'extra');
      expect(() => transport.verifyReportFiles(temp, report)).toThrow('artifact file');
      rmSync(path.join(temp, 'extra'));
      rmSync(path.join(temp, '000-test.stderr'));
      expect(() => transport.verifyReportFiles(temp, report)).toThrow();
    } finally {
      rmSync(temp, { recursive: true, force: true });
    }
  });
  it('restores exact regular files and detects removed child, source, Cargo and library bytes', async () => {
    const temp = mkdtempSync(path.join(tmpdir(), 'tmt-workspace-proof-fixture-'));
    try {
      const source = path.join(temp, 'payload');
      mkdirSync(source);
      for (const name of ['child', 'fixture', 'cargo', 'library'])
        writeFileSync(path.join(source, name), `real fixture bytes ${name}`);
      const sensitivity = transport.inputControls(
        path.join(temp, 'controls'),
        Object.fromEntries(
          ['child', 'fixture', 'cargo', 'library'].map((name) => [name, path.join(source, name)])
        )
      );
      expect(
        sensitivity.every(
          (control: {
            positive: boolean;
            missing: boolean;
            changedHash: boolean;
            restored: boolean;
          }) => control.positive && control.missing && control.changedHash && control.restored
        )
      ).toBe(true);
      const files = transport.treeFiles(source),
        archive = path.join(temp, 'closure.tar');
      const hash = await transport.packClosure(source, archive, files);
      const restored = path.join(temp, 'restored');
      await transport.unpackClosure(archive, hash, files, restored);
      transport.verifyTree(restored, files);
      for (const name of ['child', 'fixture', 'cargo', 'library']) {
        const original = readFileSync(path.join(restored, name));
        rmSync(path.join(restored, name));
        expect(() => transport.verifyTree(restored, files)).toThrow('closure input');
        writeFileSync(path.join(restored, name), original);
        transport.verifyTree(restored, files);
      }
      writeFileSync(path.join(restored, 'child'), 'corrupt');
      expect(() => transport.verifyTree(restored, files)).toThrow('closure input');
      await expect(
        transport.unpackClosure(archive, '0'.repeat(64), files, path.join(temp, 'wrong'))
      ).rejects.toThrow('Archive hash');
      await expect(
        transport.unpackClosure(archive, hash, files.slice(1), path.join(temp, 'extra'))
      ).rejects.toThrow('archive entry');
      await expect(transport.unpackClosure(archive, hash, files, restored)).rejects.toThrow(
        'absent'
      );
      symlinkSync('/etc/passwd', path.join(source, 'escape'));
      expect(() => transport.treeFiles(source)).toThrow('symlink');
    } finally {
      rmSync(temp, { recursive: true, force: true });
    }
  });
  it('refuses tar duplicate, traversal, links, extra entries and declared bound violations', async () => {
    const temp = mkdtempSync(path.join(tmpdir(), 'tmt-workspace-proof-bad-'));
    try {
      writeFileSync(path.join(temp, 'file'), 'bytes');
      const files = [{ path: 'file', ...transport.hashFile(path.join(temp, 'file')) }];
      const archive = path.join(temp, 'duplicate.tar');
      await tar.c({ cwd: temp, file: archive, portable: true }, ['file', 'file']);
      const hash = proof.digest(readFileSync(archive));
      await expect(
        transport.unpackClosure(archive, hash, files, path.join(temp, 'out'))
      ).rejects.toThrow('Duplicate');
      for (const bad of ['../escape', '/absolute', 'a//b', 'a/./b', 'a\\b'])
        expect(() => proof.validateFiles([{ ...files[0], path: bad }])).toThrow('Unsafe');
      expect(() =>
        proof.validateFiles([{ ...files[0], size: proof.LIMITS.fileBytes + 1 }])
      ).toThrow('size');
      expect(() => proof.validateFiles([{ ...files[0], mode: 0o4755 }])).toThrow('mode');
      expect(() => proof.validateFiles([{ ...files[0], sha256: 'bad' }])).toThrow('hash');
    } finally {
      rmSync(temp, { recursive: true, force: true });
    }
  });
});

it('keeps the required worker bytes and ordinary PR selection outside the manual proof', () => {
  const ci = readFileSync(path.join(root, '.github/workflows/ci.yml'), 'utf8');
  const reusable = readFileSync(path.join(root, '.github/workflows/workspace-proof.yml'), 'utf8');
  expect(ci).toContain("if: github.event_name == 'workflow_dispatch' && inputs.workspace_proof");
  expect(ci).toContain("if: steps.event.outputs.verify != 'true' && !inputs.workspace_proof");
  expect(ci).toContain('native_scope=none');
  expect(reusable).toContain('workflow_call:');
  expect(reusable).not.toContain('pull_request:');
  expect(reusable).toContain('needs: [producer, baseline]');
  expect(reusable).toContain('needs: [producer, consumer]');
  expect(reusable.match(/timeout-minutes: 20/g)).toHaveLength(5);
  expect(reusable).not.toMatch(/continue-on-error|retry|rust-cache|matrix:/);
  expect(ci).toContain('name: Native Rust workspace tests');
  expect(ci).toContain('remote_tests="$(cargo test --locked -p tmt-remote -- --list)"');
});
