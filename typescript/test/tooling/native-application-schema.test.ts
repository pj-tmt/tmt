import { createHash } from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { afterEach, describe, expect, it } from 'vite-plus/test';
import {
  CLI_SCHEMA_TARGETS,
  assertCompiledSource,
  attachApplicationSchema,
  captureSchema,
  parseCompiledSchema,
  readSchemaEvidence,
  verifyApplicationSchema,
  type ApplicationSchema,
  type ExecuteSchema,
  type TargetSchema,
} from '../../scripts/native-application-schema.mjs';
import type { VersionSnapshot } from '../../scripts/release-version-injection.mjs';

const { nativeHostTarget } = (await import(
  new URL('../../scripts/native-runtime-proof.mjs', import.meta.url).href
)) as { nativeHostTarget: () => string };
const { selectNativeArtifact } = (await import(
  new URL('../../scripts/native-artifact-policy.mjs', import.meta.url).href
)) as {
  selectNativeArtifact: (manifest: string, archive: string, target: string) => { version: string };
};

const hash = (bytes: string | Buffer) => createHash('sha256').update(bytes).digest('hex');
const roots: string[] = [];
afterEach(() =>
  roots.splice(0).forEach((root) => fs.rmSync(root, { recursive: true, force: true }))
);

function fixture() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'schema-carrier-fixture-'));
  roots.push(root);
  const hashes: Record<string, string> = {};
  for (const name of [
    'migrations.rs',
    'migrations/host_names.rs',
    'schema/001.sql',
    'schema/006_indexes.sql',
  ]) {
    const file = `rust/crates/tmt-adapters/src/storage/${name}`;
    const bytes = `Synthetic closure input: ${name}\n`;
    fs.mkdirSync(path.dirname(path.join(root, file)), { recursive: true });
    fs.writeFileSync(path.join(root, file), bytes);
    hashes[file] = hash(bytes);
  }
  hashes['rust/Cargo.toml'] = hash('original version manifest');
  hashes['rust/Cargo.lock'] = hash('original lock');
  const snapshot: VersionSnapshot = {
    schema: 1,
    cut: 'a'.repeat(40),
    product: 'cli',
    tag: 'v5.0.0-alpha.1',
    version: '5.0.0-alpha.1',
    oldVersion: '5.0.0-dev',
    manifest: 'rust/Cargo.toml',
    section: 'workspace.package',
    source: '',
    lock: '',
    packages: ['tmt-cli'],
    hashes,
  };
  // Synthetic data tests transport/closure equality; it is not copied private output or native proof.
  const record: ApplicationSchema = {
    schema_version: 1,
    product: 'cli',
    source_sha: snapshot.cut,
    databases: [{ domain: 'tmt-core-db', version: 48 }],
    source_files: Object.entries(hashes)
      .filter(([name]) => name.includes('/storage/'))
      .sort(([a], [b]) => (a < b ? -1 : 1))
      .map(([name, sha256]) => ({ path: name, sha256 })),
  };
  const executable = path.join(root, 'synthetic-binary');
  fs.writeFileSync(executable, 'Synthetic bytes; never executable\n');
  const execute: ExecuteSchema = (file, args, options) => {
    roots.push(path.dirname(options.cwd));
    expect(file).toBe(executable);
    expect(args).toEqual(['__native-schema', '--source-sha', snapshot.cut, '--json']);
    expect(options.timeoutMs).toBe(10_000);
    expect(Object.keys(options.env).sort()).toEqual([
      'HOME',
      'LANG',
      'PATH',
      'TMPDIR',
      'XDG_CONFIG_HOME',
    ]);
    return `${JSON.stringify(record)}\n`;
  };
  const manifest = {
    announcement_tag: snapshot.tag,
    artifacts: {} as Record<string, unknown>,
    releases: [{ app_name: 'tmt-cli', app_version: snapshot.version, artifacts: [] as string[] }],
  };
  const evidence: TargetSchema[] = [];
  for (const target of CLI_SCHEMA_TARGETS) {
    const name = `tmt-cli-${target}.tar.gz`;
    const archive = path.join(root, name);
    fs.writeFileSync(archive, `Synthetic archive bytes for ${target}\n`);
    manifest.artifacts[name] = {
      kind: 'executable-zip',
      name,
      target_triples: [target],
      checksums: { sha256: hash(fs.readFileSync(archive)) },
      assets: [
        'tmt',
        'tmt-driver-herdr',
        'LICENSE',
        'NATIVE-INSTALL.md',
        'THIRD-PARTY-NOTICES.txt',
      ].map((p) => ({ path: p })),
    };
    manifest.releases[0].artifacts.push(name);
    const captured = captureSchema(
      { executable, archive, target: nativeHostTarget(), snapshot, root },
      execute
    );
    evidence.push({ ...captured, target });
  }
  fs.writeFileSync(path.join(root, 'dist-manifest.json'), JSON.stringify(manifest));
  const input = { manifest, evidence, snapshot, root, directory: root };
  const attach = () => attachApplicationSchema(input);
  const target = nativeHostTarget();
  const selected = evidence.find((e) => e.target === target)!;
  const verify = (updated = attach(), exporter = execute) =>
    verifyApplicationSchema(
      {
        manifestBytes: Buffer.from(`${JSON.stringify(updated, null, 2)}\n`),
        executable,
        archive: path.join(root, `tmt-cli-${target}.tar.gz`),
        evidence: selected,
        target,
        snapshot,
        root,
      },
      exporter
    );
  return { root, record, snapshot, executable, execute, evidence, manifest, input, attach, verify };
}

describe('compiled CLI schema carrier', () => {
  it('binds four records and independently checks the final field, archived bytes and exporter output', () => {
    const f = fixture();
    const before = structuredClone(f.manifest);
    const updated = f.attach();
    expect(updated.tmt_application_schema).toEqual(f.record);
    expect(f.manifest).toEqual(before);
    const { tmt_application_schema: _, ...cargo } = updated;
    expect(cargo).toEqual(before);
    expect(f.verify(updated).record).toEqual(f.record);
  });

  it('preserves version-only preparation without treating modified Cargo files as migration inputs', () => {
    const f = fixture();
    fs.writeFileSync(path.join(f.root, 'rust/Cargo.toml'), 'Injected version manifest');
    fs.writeFileSync(path.join(f.root, 'rust/Cargo.lock'), 'Implied lock changes');
    expect(() => assertCompiledSource(f.record, f.snapshot, f.root)).not.toThrow();
    expect(f.record.databases[0].version).toBe(48); // Core output, not a count of the two SQL fixtures.
  });

  it.each(['duplicate', 'whitespace', 'extra-record', 'no-newline', 'malformed'])(
    'refuses noncanonical exporter bytes: %s',
    (kind) => {
      const f = fixture();
      const compact = JSON.stringify(f.record);
      const text = {
        duplicate: compact.replace('"schema_version":1', '"schema_version":1,"schema_version":1'),
        whitespace: `${JSON.stringify(f.record, null, 2)}\n`,
        'extra-record': `${compact}\n${compact}\n`,
        'no-newline': compact,
        malformed: '{\n',
      }[kind]!;
      expect(() => parseCompiledSchema(text)).toThrow();
    }
  );

  it.each([
    'unknown-field',
    'missing-field',
    'schema-revision',
    'product',
    'domain',
    'version',
    'overflow-version',
    'source',
    'missing-input',
    'extra-input',
    'duplicate-path',
    'unsorted-paths',
    'wrong-hash',
    'escape',
  ])('refuses wrong record identity/closure: %s', (kind) => {
    const f = fixture();
    const record = structuredClone(f.record) as ApplicationSchema & { unexpected?: boolean };
    if (kind === 'unknown-field') record.unexpected = true;
    if (kind === 'missing-field') delete (record as Partial<ApplicationSchema>).source_sha;
    if (kind === 'schema-revision') Object.assign(record, { schema_version: 2 });
    if (kind === 'product') Object.assign(record, { product: 'remote' });
    if (kind === 'domain') Object.assign(record.databases[0], { domain: 'tmt-remote-db' });
    if (kind === 'version') record.databases[0].version = -1;
    if (kind === 'overflow-version') record.databases[0].version = 2_147_483_648;
    if (kind === 'source') record.source_sha = 'b'.repeat(40);
    if (kind === 'missing-input') record.source_files.pop();
    if (kind === 'extra-input')
      record.source_files.push({ path: 'untracked.sql', sha256: 'b'.repeat(64) });
    if (kind === 'duplicate-path') record.source_files[1] = record.source_files[0];
    if (kind === 'unsorted-paths') record.source_files.reverse();
    if (kind === 'wrong-hash') record.source_files[0].sha256 = 'b'.repeat(64);
    if (kind === 'escape') record.source_files[0].path = '../outside';
    expect(() =>
      assertCompiledSource(parseCompiledSchema(`${JSON.stringify(record)}\n`), f.snapshot, f.root)
    ).toThrow();
  });

  it('refuses changed on-disk source and new undeclared captured inputs instead of parsing Rust', () => {
    const f = fixture();
    fs.writeFileSync(path.join(f.root, f.record.source_files[0].path), 'changed');
    expect(() => f.attach()).toThrow('Prepared schema source changed');
    const other = fixture();
    other.snapshot.hashes['rust/crates/tmt-adapters/src/storage/schema/extra.sql'] = 'b'.repeat(64);
    expect(() => other.attach()).toThrow('Different preparation source snapshot');
  });

  it.each([
    'omitted',
    'duplicate',
    'disagree',
    'snapshot',
    'output',
    'binary',
    'archive',
    'unknown-field',
  ])('refuses broken target/provenance evidence: %s', (kind) => {
    const f = fixture();
    if (kind === 'omitted') f.evidence.pop();
    if (kind === 'duplicate') f.evidence[1].target = f.evidence[0].target;
    if (kind === 'disagree') {
      f.evidence[1].record = structuredClone(f.record);
      f.evidence[1].record.databases[0].version++;
      f.evidence[1].output_sha256 = hash(`${JSON.stringify(f.evidence[1].record)}\n`);
    }
    if (kind === 'snapshot') f.evidence[1].source_snapshot_sha256 = 'b'.repeat(64);
    if (kind === 'output') f.evidence[1].output_sha256 = 'b'.repeat(64);
    if (kind === 'binary') f.evidence[1].binary_sha256 = '';
    if (kind === 'archive') f.evidence[1].archive_sha256 = 'b'.repeat(64);
    if (kind === 'unknown-field') Object.assign(f.evidence[1], { arbitrary: true });
    expect(() => f.attach()).toThrow();
  });

  it('refuses missing/changed final fields and different independently exported archived schema', () => {
    const f = fixture();
    expect(() => f.verify(f.manifest)).toThrow('lost application schema');
    const updated = structuredClone(f.attach());
    (updated.tmt_application_schema as ApplicationSchema).databases[0].version++;
    expect(() => f.verify(updated)).toThrow('Manifest schema differs from target capture');
    const g = fixture();
    expect(() =>
      g.verify(g.attach(), (file, args, opts) => {
        const record = JSON.parse(g.execute(file, args, opts));
        record.databases[0].version++;
        return `${JSON.stringify(record)}\n`;
      })
    ).toThrow('Archived CLI schema differs');
  });

  it('refuses duplicate keys in final metadata and altered target binary digests', () => {
    const f = fixture();
    const bytes = `${JSON.stringify(f.attach(), null, 2)}\n`.replace(
      '"schema_version": 1',
      '"schema_version": 1,"schema_version": 1'
    );
    expect(() =>
      verifyApplicationSchema(
        {
          manifestBytes: Buffer.from(bytes),
          executable: f.executable,
          archive: path.join(f.root, `tmt-cli-${nativeHostTarget()}.tar.gz`),
          evidence: f.evidence[0],
          target: nativeHostTarget(),
          snapshot: f.snapshot,
          root: f.root,
        },
        f.execute
      )
    ).toThrow('encoding changed');
    f.evidence.find((e) => e.target === nativeHostTarget())!.binary_sha256 = 'b'.repeat(64);
    expect(() => f.verify()).toThrow('Archived binary differs');
  });

  it('preserves original exporter failure and refuses state writes using injected zero-child controls', () => {
    const f = fixture();
    const archive = path.join(f.root, `tmt-cli-${nativeHostTarget()}.tar.gz`);
    const call = (execute: ExecuteSchema) =>
      captureSchema(
        {
          executable: f.executable,
          archive,
          target: nativeHostTarget(),
          snapshot: f.snapshot,
          root: f.root,
        },
        execute
      );
    expect(() =>
      call((file, args, opts) => {
        f.execute(file, args, opts);
        throw new Error('original unavailable');
      })
    ).toThrow('no legacy preparation fallback');
    expect(() =>
      call((file, args, opts) => {
        const out = f.execute(file, args, opts);
        fs.writeFileSync(path.join(opts.env.HOME, 'state'), 'unexpected');
        return out;
      })
    ).toThrow('wrote state');
    try {
      call((file, args, opts) => {
        f.execute(file, args, opts);
        fs.writeFileSync(path.join(opts.env.HOME, 'state'), 'unexpected');
        throw new Error('original unavailable');
      });
      expect.unreachable();
    } catch (error) {
      expect(error).toBeInstanceOf(AggregateError);
      expect((error as AggregateError).errors[0].cause.message).toBe('original unavailable');
      expect((error as AggregateError).errors[1].message).toContain('wrote state');
    }
  });

  it('refuses an executable changed during export rather than binding its later bytes', () => {
    const f = fixture();
    expect(() =>
      f.verify(f.attach(), (file, args, opts) => {
        const output = f.execute(file, args, opts);
        fs.writeFileSync(file, 'Altered binary bytes');
        return output;
      })
    ).toThrow('Prepared schema executable changed during export');
  });

  it('rejects duplicate/extra sidecar records before either consumer can lose raw evidence', () => {
    const f = fixture();
    const file = path.join(f.root, 'schema-evidence.json');
    const compact = JSON.stringify(f.evidence[0]);
    fs.writeFileSync(file, `${compact}\n`);
    expect(readSchemaEvidence(file)).toEqual(f.evidence[0]);
    fs.writeFileSync(
      file,
      `${compact.replace('"schema_version":1', '"schema_version":1,"schema_version":1')}\n`
    );
    expect(() => readSchemaEvidence(file)).toThrow('Target evidence encoding changed');
    fs.writeFileSync(file, `${compact}\n${compact}\n`);
    expect(() => readSchemaEvidence(file)).toThrow();
  });

  it('refuses output/input limits and a foreign host without invoking the exporter', () => {
    const f = fixture();
    expect(() => parseCompiledSchema(' '.repeat(1024 * 1024 + 1))).toThrow('Schema output bound');
    const record = structuredClone(f.record);
    record.source_files = Array.from({ length: 65 }, (_, i) => ({
      path: `input${i}`,
      sha256: 'b'.repeat(64),
    }));
    expect(() => parseCompiledSchema(`${JSON.stringify(record)}\n`)).toThrow(
      'Compiled source closure bound'
    );
    const target = CLI_SCHEMA_TARGETS.find((t) => t !== nativeHostTarget())!;
    let invoked = false;
    expect(() =>
      captureSchema(
        {
          executable: f.executable,
          archive: path.join(f.root, `tmt-cli-${target}.tar.gz`),
          target,
          snapshot: f.snapshot,
          root: f.root,
        },
        () => {
          invoked = true;
          return '';
        }
      )
    ).toThrow('matching native host');
    expect(invoked).toBe(false);
  });

  it('retains schema-less legacy manifest selection and refuses a second schema insertion', () => {
    const f = fixture();
    const target = nativeHostTarget();
    expect(
      selectNativeArtifact(
        path.join(f.root, 'dist-manifest.json'),
        path.join(f.root, `tmt-cli-${target}.tar.gz`),
        target
      ).version
    ).toBe(f.snapshot.version);
    const updated = f.attach();
    fs.writeFileSync(path.join(f.root, 'dist-manifest.json'), JSON.stringify(updated));
    expect(() => attachApplicationSchema({ ...f.input, manifest: updated })).toThrow(
      'already exists'
    );
  });
});
