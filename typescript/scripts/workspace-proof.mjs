// N=1 admission only. Cargo and libtest own discovery; unknown formats refuse proof.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import path from 'node:path';

export const TOOLCHAIN = '1.97.0';
export const EXCLUSIONS = [
  'tmt-office',
  'tmt-office-storage',
  'tmt-office-pairing',
  'tmt-office-service',
];
export const LIMITS = Object.freeze({
  files: 150_000,
  bytes: 5 * 1024 ** 3,
  fileBytes: 512 * 1024 ** 2,
  archiveBytes: 5 * 1024 ** 3 + 150_000 * 4096,
  outputBytes: 32 * 1024 ** 2,
  evidenceBytes: 256 * 1024 ** 2,
  commands: 4000,
  commandMs: 18 * 60_000,
});
export const ROLES = ['baseline', 'producer', 'consumer', 'doctest'];
export function digest(value) {
  return createHash('sha256').update(value).digest('hex');
}
export function canonical(value) {
  if (Array.isArray(value)) return `[${value.map(canonical).join(',')}]`;
  if (value && typeof value === 'object')
    return `{${Object.keys(value)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${canonical(value[key])}`)
      .join(',')}}`;
  return JSON.stringify(value);
}
export function cargoArgs(action, extra = []) {
  assert(['build', 'test'].includes(action), 'Unsupported Cargo proof action');
  return [
    `+${TOOLCHAIN}`,
    action,
    '--locked',
    '--workspace',
    ...EXCLUSIONS.flatMap((name) => ['--exclude', name]),
    ...extra,
  ];
}
export function exact(actual, expected, label) {
  assert.equal(canonical(actual), canonical(expected), label);
}
function unique(values, label) {
  assert.equal(new Set(values).size, values.length, `Duplicate ${label}`);
  return [...values].sort();
}
export function relativeFile(value) {
  assert(
    typeof value === 'string' &&
      value.length > 0 &&
      value.length <= 1024 &&
      !value.includes('\\') &&
      ![...value].some(
        (character) => character.charCodeAt(0) < 32 || character.charCodeAt(0) === 127
      ) &&
      !path.posix.isAbsolute(value) &&
      value.split('/').every((part) => part && part !== '.' && part !== '..'),
    'Unsafe closure path'
  );
  return value;
}
export function validateFiles(files) {
  assert(
    Array.isArray(files) && files.length > 0 && files.length <= LIMITS.files,
    'Closure file count'
  );
  unique(
    files.map((file) => relativeFile(file.path)),
    'closure path'
  );
  let bytes = 0;
  for (const file of files) {
    assert(
      Number.isSafeInteger(file.size) && file.size >= 0 && file.size <= LIMITS.fileBytes,
      'Closure file size'
    );
    assert(Number.isInteger(file.mode) && file.mode >= 0 && file.mode <= 0o777, 'Closure mode');
    assert(/^[a-f0-9]{64}$/.test(file.sha256), 'Closure hash');
    bytes += file.size;
  }
  assert(bytes <= LIMITS.bytes, 'Closure byte count');
  return bytes;
}
export function cargoOutput(stdout, { mixed = false } = {}) {
  assert(Buffer.byteLength(stdout) <= LIMITS.outputBytes, 'Cargo output bound');
  const records = [],
    text = [];
  for (const line of stdout.split('\n')) {
    if (line.startsWith('{')) {
      let record;
      try {
        record = JSON.parse(line);
      } catch {
        assert.fail('Ambiguous Cargo JSON');
      }
      assert(
        [
          'compiler-artifact',
          'compiler-message',
          'build-script-executed',
          'build-finished',
        ].includes(record.reason),
        'Unknown Cargo record'
      );
      records.push(record);
    } else if (line.trim()) {
      assert(mixed, 'Unexpected non-JSON Cargo output');
      text.push(line);
    }
  }
  const finished = records.filter((record) => record.reason === 'build-finished');
  assert(
    finished.length === 1 && finished[0].success === true && records.at(-1) === finished[0],
    'Cargo compilation did not finish successfully'
  );
  return { records, text: text.join('\n') };
}
export function unitKey(record) {
  return canonical([
    record.package_id,
    record.target.src_path,
    record.target.name,
    record.target.kind,
    record.target.crate_types,
    record.profile,
  ]);
}
export function libraryFeatures(records) {
  const values = records
    .filter(
      (record) =>
        record.reason === 'compiler-artifact' &&
        !record.profile.test &&
        record.target.kind.some((kind) => ['lib', 'rlib', 'proc-macro'].includes(kind))
    )
    .map((record) => [unitKey(record), unique(record.features, 'feature')]);
  unique(
    values.map(([key]) => key),
    'library unit'
  );
  return values.sort(([a], [b]) => a.localeCompare(b));
}
export function compareDocFeatures(baseline, docs) {
  const base = new Map(baseline);
  assert(docs.length > 0, 'Missing doctest library units');
  for (const [key, features] of docs) {
    assert(base.has(key), 'Doctest unit absent from default graph');
    exact(features, base.get(key), 'Doctest feature mismatch');
  }
}
export function admitInventory(records, packages, manifests) {
  assert(packages.length > EXCLUSIONS.length, 'Incomplete workspace metadata');
  for (const name of EXCLUSIONS)
    assert(
      packages.some((pkg) => pkg.name === name),
      `Missing exclusion ${name}`
    );
  const selected = packages.filter((pkg) => !EXCLUSIONS.includes(pkg.name));
  const byId = new Map(packages.map((pkg) => [pkg.id, pkg]));
  assert(!byId.has(undefined) && byId.size === packages.length, 'Missing Cargo package identity');
  const artifacts = records.filter((record) => record.reason === 'compiler-artifact');
  for (const artifact of artifacts) {
    assert(
      artifact.profile &&
        typeof artifact.profile.test === 'boolean' &&
        Array.isArray(artifact.features) &&
        artifact.features.every((feature) => typeof feature === 'string') &&
        Array.isArray(artifact.filenames) &&
        artifact.filenames.length > 0 &&
        artifact.filenames.every((file) => typeof file === 'string' && path.isAbsolute(file)) &&
        (artifact.executable === null ||
          (typeof artifact.executable === 'string' &&
            artifact.filenames.includes(artifact.executable))),
      'Incomplete Cargo artifact record'
    );
    const pkg = byId.get(artifact.package_id);
    if (pkg)
      assert(
        pkg.targets.some(
          (target) =>
            target.name === artifact.target.name &&
            target.src_path === artifact.target.src_path &&
            canonical(target.kind) === canonical(artifact.target.kind)
        ),
        'Unknown workspace artifact target'
      );
  }
  unique(
    artifacts.map((record) => canonical([unitKey(record), record.features])),
    'Cargo artifact unit'
  );
  const harnesses = [],
    support = [],
    docs = [];
  for (const pkg of selected) {
    assert(Array.isArray(pkg.targets) && pkg.targets.length, 'Missing Cargo targets');
    const manifest = manifests[pkg.name];
    assert(manifest?.package, 'Missing Cargo-owned manifest');
    for (const target of pkg.targets) {
      assert(target.kind.length === 1, 'Unsupported multi-kind target');
      const kind = target.kind[0];
      assert(
        ['lib', 'bin', 'test', 'example', 'bench', 'custom-build', 'proc-macro'].includes(kind),
        'Unsupported target kind'
      );
      const settings =
        kind === 'lib' || kind === 'proc-macro'
          ? (manifest.lib ?? {})
          : ((manifest[kind] ?? []).find(
              (entry) =>
                entry.name === target.name ||
                (entry.path &&
                  path.resolve(path.dirname(pkg.manifestPath), entry.path) === target.src_path)
            ) ?? {});
      assert(settings.harness !== false, 'Unsupported harness=false target');
      assert(
        !(target['required-features']?.length || settings['required-features']?.length),
        'Required-feature target needs an explicit disposition'
      );
      const units = artifacts.filter(
        (record) =>
          record.package_id === pkg.id &&
          record.target.name === target.name &&
          record.target.src_path === target.src_path &&
          canonical(record.target.kind) === canonical(target.kind)
      );
      const tests = units.filter((record) => record.profile.test && record.executable);
      const runnable = target.test && !['bench', 'custom-build'].includes(kind);
      if (runnable) {
        assert.equal(tests.length, 1, `Missing/ambiguous harness ${pkg.name}/${target.name}`);
        const record = tests[0];
        assert(path.isAbsolute(record.executable), 'Harness path must be absolute');
        harnesses.push({
          id: unitKey(record),
          package: pkg.name,
          cwd: path.dirname(pkg.manifestPath),
          executable: record.executable,
          features: unique(record.features, 'feature'),
          target,
        });
      } else assert.equal(tests.length, 0, 'Unexpected executable test unit');
      if (kind !== 'bench')
        assert(units.length > 0, `Missing compile/support target ${pkg.name}/${target.name}`);
      support.push({
        package: pkg.name,
        target,
        disposition: runnable ? 'harness' : kind === 'bench' ? 'not-default' : 'compile-support',
        units: units
          .map((record) => ({
            key: unitKey(record),
            features: record.features,
            files: record.filenames,
            executable: record.executable,
          }))
          .sort((a, b) => a.key.localeCompare(b.key)),
      });
      if (['lib', 'proc-macro'].includes(kind) && target.doctest)
        docs.push({
          id: pkg.id,
          package: pkg.name,
          target: target.name,
          cwd: path.dirname(pkg.manifestPath),
        });
    }
  }
  unique(
    harnesses.map((harness) => harness.executable),
    'harness executable'
  );
  assert(
    harnesses.some(
      (harness) =>
        harness.package === 'tmt-cli' &&
        harness.target.name === 'architecture' &&
        harness.target.src_path.endsWith('/tests/architecture.rs')
    ),
    'Architecture harness missing'
  );
  for (const name of ['tmt-office-command', 'tmt-office-model', 'tmt-remote'])
    assert(
      harnesses.some((harness) => harness.package === name),
      `Retained package missing: ${name}`
    );
  return {
    harnesses: harnesses.sort((a, b) => a.id.localeCompare(b.id)),
    support: support.sort((a, b) =>
      canonical([a.package, a.target.name]).localeCompare(canonical([b.package, b.target.name]))
    ),
    docs: docs.sort((a, b) => a.package.localeCompare(b.package)),
    libraryFeatures: libraryFeatures(records),
  };
}
export function parseList(text) {
  const names = [];
  let summary;
  for (const line of text.split('\n').filter(Boolean)) {
    const count = /^(\d+) tests?, (\d+) benchmarks?$/.exec(line);
    if (count) {
      assert(summary === undefined && Number(count[2]) === 0, 'Ambiguous list summary/benchmark');
      summary = Number(count[1]);
    } else {
      const name = /^(.*): test$/.exec(line);
      assert(name && name[1] && !summary && summary !== 0, 'Unknown list output');
      names.push(name[1]);
    }
  }
  assert(summary !== undefined && summary === names.length, 'Incomplete list');
  return unique(names, 'listed test');
}
export function enumeration(normal, ignored) {
  const names = parseList(normal),
    ignoredNames = parseList(ignored);
  assert(
    ignoredNames.every((name) => names.includes(name)),
    'Ignored list is not a subset'
  );
  return { names, ignored: ignoredNames };
}
export function parseExecution(text, expected) {
  const lines = text.split('\n').filter(Boolean);
  const started = /^running (\d+) tests?$/.exec(lines.shift() ?? '');
  assert(started && Number(started[1]) === expected.names.length, 'Incomplete execution start');
  const dispositions = new Map();
  let summary;
  for (const line of lines) {
    const result =
      /^test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out; finished in [0-9.]+s$/.exec(
        line
      );
    if (result) {
      assert(!summary && result[1] === 'ok', 'Failed/duplicate execution summary');
      summary = result;
      continue;
    }
    const terminal = /^test (.+) \.\.\. (ok|FAILED|ignored)(?:, .*)?$/.exec(line);
    if (terminal) {
      assert(!summary && !dispositions.has(terminal[1]), 'Duplicate/late test record');
      dispositions.set(terminal[1], terminal[2]);
      continue;
    }
    assert(
      !summary && /^test .+ has been running for over \d+ seconds$/.test(line),
      'Ambiguous execution output'
    );
  }
  assert(summary, 'Missing execution summary');
  exact([...dispositions.keys()].sort(), expected.names, 'Missing/extra test record');
  exact(
    [...dispositions]
      .filter(([, value]) => value === 'ignored')
      .map(([name]) => name)
      .sort(),
    expected.ignored,
    'Changed ignored disposition'
  );
  assert(
    [...dispositions.values()].every((value) => value !== 'FAILED'),
    'Failed test'
  );
  exact(
    summary.slice(2, 7).map(Number),
    [expected.names.length - expected.ignored.length, 0, expected.ignored.length, 0, 0],
    'Filtered/incomplete summary'
  );
  return [...dispositions].sort(([a], [b]) => a.localeCompare(b));
}
export function sections(text, mode) {
  const end = mode === 'list' ? /^\d+ tests?, \d+ benchmarks?$/ : /^test result:/;
  const result = [];
  let pending = [];
  for (const line of text.split('\n')) {
    if (!line.trim()) continue;
    pending.push(line);
    if (end.test(line)) {
      result.push(pending.join('\n'));
      pending = [];
    }
  }
  assert(pending.length === 0, 'Truncated/unknown Cargo test output');
  return result;
}
export function cargoCoverage(stdout, stderr, inventory, mode, ignored = false) {
  const parsed = cargoOutput(stdout, { mixed: true });
  const blocks = sections(parsed.text, mode);
  const paths = [...stderr.matchAll(/^\s*Running .*\(([^\n()]+)\)$/gm)].map((match) =>
    path.resolve(inventory.rustRoot, match[1])
  );
  unique(paths, 'Cargo executed binary');
  const byExecutable = new Map(inventory.harnesses.map((harness) => [harness.executable, harness]));
  for (const executable of paths)
    assert(byExecutable.has(executable), 'Cargo ran an unadmitted executable');
  const ordinary = paths.map((executable, index) => {
    const harness = byExecutable.get(executable);
    assert(blocks[index] !== undefined, 'Missing Cargo binary block');
    return {
      id: harness.id,
      values:
        mode === 'list' ? parseList(blocks[index]) : parseExecution(blocks[index], harness.list),
    };
  });
  const headers = [...stderr.matchAll(/^\s*Doc-tests (\S+)\s*$/gm)].map((match) => match[1]);
  unique(headers, 'doctest header');
  exact(
    headers.sort(),
    inventory.docs.map((doc) => doc.target).sort(),
    'Missing/extra doctest target'
  );
  const docs = new Map(inventory.docs.map((doc) => [doc.package, []]));
  for (const block of blocks.slice(paths.length)) {
    const values = mode === 'list' ? parseList(block) : executionNames(block);
    const names = mode === 'list' ? values : values.names;
    // Rustdoc 2024 may merge blocks. Attribute actual names by Cargo's package cwd,
    // not Markdown parsing or a guessed number of generated executables.
    const members = names.map((name) => {
      const source = /^(.*?) - .* \(line \d+\)$/.exec(name);
      assert(source, 'Unsupported doctest name');
      const file = path.resolve(inventory.rustRoot, source[1]);
      const owners = inventory.docs.filter((doc) => {
        const rel = path.relative(doc.cwd, file);
        return rel && !rel.startsWith('../') && !path.isAbsolute(rel);
      });
      assert(owners.length === 1, 'Ambiguous doctest source owner');
      return owners[0].package;
    });
    if (mode === 'execution') {
      const expected = {
        names: [...names].sort(),
        ignored: names.filter((name) => inventory.docLists[name]?.ignored === true).sort(),
      };
      parseExecution(block, expected);
    }
    names.forEach((name, index) =>
      docs.get(members[index]).push(mode === 'list' ? name : [name, values.dispositions.get(name)])
    );
  }
  if (mode === 'execution')
    exact(
      [...docs.values()]
        .flat()
        .map(([name]) => name)
        .sort(),
      Object.keys(inventory.docLists).sort(),
      'Missing/extra doctest execution'
    );
  const docValues = [...docs]
    .map(([pkg, values]) => {
      unique(
        values.map((value) => (typeof value === 'string' ? value : value[0])),
        'doctest name'
      );
      return {
        package: pkg,
        values: values.sort((a, b) => canonical(a).localeCompare(canonical(b))),
        ignored,
      };
    })
    .sort((a, b) => a.package.localeCompare(b.package));
  const features = libraryFeatures(parsed.records);
  for (const doc of inventory.docs)
    if (doc.id)
      assert(
        features.some(([key]) => {
          const unit = JSON.parse(key);
          return unit[0] === doc.id && unit[2] === doc.target;
        }),
        'Missing doctest primary library unit'
      );
  return {
    ordinary: ordinary.sort((a, b) => a.id.localeCompare(b.id)),
    docs: docValues,
    zeroDocSections: blocks
      .slice(paths.length)
      .filter((block) =>
        mode === 'list' ? parseList(block).length === 0 : executionNames(block).names.length === 0
      ).length,
    libraryFeatures: features,
  };
}
function executionNames(block) {
  const dispositions = new Map(
    [...block.matchAll(/^test (.+) \.\.\. (ok|FAILED|ignored)(?:, .*)?$/gm)].map((match) => [
      match[1],
      match[2],
    ])
  );
  return { names: [...dispositions.keys()], dispositions };
}
export function assignment(harnesses) {
  const ids = unique(
    harnesses.map((harness) => harness.id),
    'assigned binary'
  );
  assert(ids.length > 0, 'Empty N=1 assignment');
  return [{ index: 0, ids }];
}
export function aggregate(reports, results) {
  exact(Object.keys(results).sort(), [...ROLES].sort(), 'Missing/extra job result');
  assert(
    ROLES.every((role) => results[role] === 'success'),
    'Failed/cancelled/skipped proof job'
  );
  exact(
    reports.map((report) => report.role).sort(),
    [...ROLES].sort(),
    'Missing/duplicate/extra proof report'
  );
  for (const report of reports)
    assert(
      report.identity && Array.isArray(report.commands),
      'Missing report identity/process records'
    );
  const producer = reports.find((report) => report.role === 'producer');
  assert(
    /^[a-f0-9]{64}$/.test(producer.inventoryHash) &&
      /^[a-f0-9]{64}$/.test(producer.bundleHash) &&
      Array.isArray(producer.assignment) &&
      producer.assignment.length === 1 &&
      producer.assignment[0].index === 0 &&
      producer.assignment[0].ids.length > 0,
    'Missing frozen inventory/assignment/payload'
  );
  const byRole = Object.fromEntries(reports.map((report) => [report.role, report]));
  for (const report of reports) {
    assert(
      report.complete === true && report.runtimeRootsRemoved === true,
      'Incomplete report/root cleanup'
    );
    exact(
      report.identity,
      byRole.producer.identity,
      'Source/attempt/platform/toolchain/environment mismatch'
    );
  }
  exact(byRole.consumer.bundleHash, byRole.producer.bundleHash, 'Runtime payload hash mismatch');
  for (const report of reports)
    assert(
      Array.isArray(report.commands) &&
        report.commands.length > 0 &&
        report.commands.every(
          (command) =>
            command.status === 0 && command.signal === null && command.cleanup && !command.reason
        ),
      'Missing/failed process evidence'
    );
  exact(byRole.consumer.inventoryHash, byRole.producer.inventoryHash, 'Inventory hash mismatch');
  exact(byRole.consumer.assignment, byRole.producer.assignment, 'Incomplete/extra assignment');
  exact(
    byRole.producer.assignment,
    assignment(byRole.producer.inventory.harnesses),
    'Assignment does not cover inventory'
  );
  exact(
    byRole.consumer.execution.map((record) => record.id).sort(),
    byRole.producer.assignment[0].ids,
    'Missing/extra executed binary'
  );
  exact(
    byRole.baseline.inventory,
    byRole.producer.inventory,
    'Default/producer target or feature mismatch'
  );
  exact(
    byRole.baseline.execution,
    byRole.consumer.execution,
    'Default/restored binary coverage mismatch'
  );
  exact(byRole.baseline.docs, byRole.doctest.docs, 'Default/separate doctest obligation mismatch');
  compareDocFeatures(byRole.baseline.libraryFeatures, byRole.doctest.libraryFeatures);
  assert(
    byRole.consumer.closureVerified && byRole.consumer.cleanupVerified,
    'Runtime closure/cleanup not proved'
  );
  return {
    complete: true,
    source: byRole.producer.identity.source,
    inventoryHash: byRole.producer.inventoryHash,
  };
}
