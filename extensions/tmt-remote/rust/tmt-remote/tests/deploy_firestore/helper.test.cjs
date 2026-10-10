'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const http = require('node:http');
const crypto = require('node:crypto');
const { execute, validate, run, fetchJson } = require('../../src/deploy_firestore/helper.cjs');
const ACCOUNT = 'owner@example.test',
  PROJECT = 'demo-remote-1',
  ID = '3f2b8c1e-5d4a-4e7b-9c1d-2a6f8e0b4c11';
const CANARY = 'TOKEN_CANARY_DO_NOT_DISCLOSE';
const database = `projects/${PROJECT}/databases/(default)`;
const field = `${database}/collectionGroups/log/fields/sequence`;
const release = `projects/${PROJECT}/releases/cloud.firestore`;
const hash = (s) => crypto.createHash('sha256').update(s).digest('hex');
const body = "rules_version = '2';\n// checked fixture body\n";
const source = `// tmt-remote deployment ${ID} rules ${hash(body)}\n${body}`;
function request(operation, input = {}) {
  return {
    version: 1,
    operation,
    input,
    account: operation.startsWith('apply') ? ACCOUNT : null,
    budgetMs: 30000,
  };
}
function rulesInput(extra = {}) {
  return { project: PROJECT, source, deployment: ID, replacedDigest: null, ...extra };
}
function fixture(route, account = ACCOUNT) {
  const calls = [];
  return {
    calls,
    deadline: Date.now() + 30000,
    credential: async () => ({ account, token: CANARY }),
    http: async (url, options) => {
      calls.push({
        url,
        method: options.method,
        body: options.body ? JSON.parse(options.body) : null,
      });
      if (url.endsWith('/oauth2/v3/userinfo'))
        return { status: 200, body: { email: account, email_verified: true } };
      if (url === `https://firebase.googleapis.com/v1beta1/projects/${PROJECT}`)
        return { status: 200, body: { projectId: PROJECT, state: 'ACTIVE' } };
      return route(url, options);
    },
  };
}
function mutations(f) {
  return f.calls.filter((c) => c.method !== 'GET');
}
async function failure(action, code) {
  await assert.rejects(action, (e) => e.code === code);
}

test('protocol rejects extra keys, arbitrary URLs, unbound effects and unsupported modes', () => {
  for (const r of [
    { ...request('account'), token: CANARY },
    request('apply-database', { project: '../../other', location: 'us-central1' }),
    request('fetch', { url: 'https://evil.invalid' }),
    { ...request('apply-sign-in', { project: PROJECT, provider: 'anonymous' }), account: null },
  ])
    assert.throws(() => validate(r));
});
test('live identity is verified, and changed account refuses before a project effect', async () => {
  const f = fixture(() => assert.fail('no project request'), 'changed@example.test');
  await failure(
    () => execute(request('apply-database', { project: PROJECT, location: 'us-central1' }), f),
    'account-changed'
  );
  assert.equal(mutations(f).length, 0);
});
test('read-only Rules observation returns exact bytes and does not allocate', async () => {
  const f = fixture((url) => ({
    status: 200,
    body: url.endsWith('/cloud.firestore')
      ? { rulesetName: `projects/${PROJECT}/rulesets/one` }
      : { source: { files: [{ name: 'firestore.rules', content: source }] } },
  }));
  assert.equal(await execute(request('live-rules', { project: PROJECT }), f), source);
  assert.equal(mutations(f).length, 0);
});
test('database mismatch has no effect and cannot change region or edition', async () => {
  const f = fixture(() => ({
    status: 200,
    body: {
      name: database,
      locationId: 'other',
      type: 'FIRESTORE_NATIVE',
      databaseEdition: 'ENTERPRISE',
    },
  }));
  assert.equal(
    await execute(request('observe-database', { project: PROJECT, location: 'us-central1' }), f),
    'mismatch'
  );
  await failure(
    () => execute(request('apply-database', { project: PROJECT, location: 'us-central1' }), f),
    'database-mismatch'
  );
  assert.equal(mutations(f).length, 0);
});
test('database creation is one request and lost reply remains unknown without retry', async () => {
  const f = fixture((_, o) => {
    if (o.method === 'GET') return { status: 404, body: {} };
    throw Error(CANARY);
  });
  await failure(
    () => execute(request('apply-database', { project: PROJECT, location: 'us-central1' }), f),
    'unknown'
  );
  assert.equal(mutations(f).length, 1);
  assert.deepEqual(mutations(f)[0].body, {
    locationId: 'us-central1',
    type: 'FIRESTORE_NATIVE',
    databaseEdition: 'STANDARD',
  });
});
test('Auth initialization and Google OAuth setup stay owner actions', async () => {
  for (const [provider, config, expected] of [
    ['anonymous', null, 'initialize-auth'],
    ['google.com', {}, 'enable-google-sign-in'],
  ]) {
    const f = fixture((url) =>
      url.endsWith('/config')
        ? { status: config === null ? 404 : 200, body: config }
        : { status: 404, body: {} }
    );
    assert.equal(
      await execute(request('apply-sign-in', { project: PROJECT, provider }), f),
      expected
    );
    assert.equal(mutations(f).length, 0);
  }
});
test('anonymous enable only patches the selected provider, not another setting', async () => {
  const f = fixture((url, o) => ({
    status: 200,
    body: o.method === 'GET' ? { signIn: { anonymous: { enabled: false } } } : {},
  }));
  assert.equal(
    await execute(request('apply-sign-in', { project: PROJECT, provider: 'anonymous' }), f),
    'done'
  );
  assert.deepEqual(mutations(f)[0].body, { signIn: { anonymous: { enabled: true } } });
  assert.match(mutations(f)[0].url, /updateMask=signIn.anonymous.enabled$/);
});
const indexInput = { project: PROJECT, collection: 'log', field: 'sequence', direction: 'asc' };
test('200 unrelated live field configs stop a new allocation at the shared ceiling', async () => {
  const f = fixture((url) => ({
    status: 200,
    body: url.includes('/-/fields')
      ? {
          fields: Array.from({ length: 200 }, (_, n) => ({
            name: `${database}/collectionGroups/other/fields/f${n}`,
          })),
        }
      : { name: field, indexConfig: { indexes: [], usesAncestorConfig: true } },
  }));
  await failure(() => execute(request('apply-index', indexInput), f), 'quota-exceeded');
  assert.equal(mutations(f).length, 0);
});
test('adding to an existing config at capacity preserves every index and TTL', async () => {
  const other = {
    queryScope: 'COLLECTION_GROUP',
    apiScope: 'ANY_API',
    density: 'SPARSE_ALL',
    multikey: false,
    unique: false,
    fields: [{ fieldPath: 'sequence', order: 'DESCENDING' }],
    name: 'server-name',
    state: 'READY',
  };
  const f = fixture((url, o) => ({
    status: 200,
    body:
      o.method !== 'GET'
        ? {}
        : url.includes('/-/fields')
          ? {
              fields: [
                { name: field },
                ...Array.from({ length: 199 }, (_, n) => ({
                  name: `${database}/collectionGroups/other/fields/f${n}`,
                })),
              ],
            }
          : {
              name: field,
              ttlConfig: { state: 'ACTIVE' },
              indexConfig: { indexes: [other], usesAncestorConfig: false },
            },
  }));
  assert.equal(await execute(request('apply-index', indexInput), f), 'done');
  const patch = mutations(f)[0];
  assert.equal(patch.body.indexConfig.indexes.length, 2);
  const { name, state, ...kept } = other;
  assert.deepEqual(patch.body.indexConfig.indexes[0], kept);
  assert.equal(patch.body.ttlConfig, undefined);
  assert.match(patch.url, /updateMask=indexConfig$/);
});
test('an existing building index is observed without another patch', async () => {
  const f = fixture(() => ({
    status: 200,
    body: {
      name: field,
      indexConfig: {
        indexes: [
          {
            queryScope: 'COLLECTION',
            state: 'CREATING',
            fields: [{ fieldPath: 'sequence', order: 'ASCENDING' }],
          },
        ],
      },
    },
  }));
  assert.equal(await execute(request('observe-index', indexInput), f), 'building');
  assert.equal(mutations(f).length, 0);
});
function rulesFixture(options = {}) {
  let live = options.live ?? null,
    made = options.made ?? [],
    serial = 0;
  const f = fixture((url, o) => {
    const route = new URL(url).pathname.replace(/^\/v1\//, '');
    if (route === release && o.method === 'GET')
      return { status: live === null ? 404 : 200, body: { rulesetName: live } };
    if (route.startsWith(`projects/${PROJECT}/rulesets/`) && o.method === 'GET') {
      const row = made.find((r) => r.name === route);
      return { status: 200, body: row };
    }
    if (route === `projects/${PROJECT}/rulesets` && o.method === 'GET')
      return { status: 200, body: { rulesets: made.map((x) => ({ name: x.name })) } };
    if (route === `projects/${PROJECT}/rulesets` && o.method === 'POST') {
      const row = { name: `projects/${PROJECT}/rulesets/new${++serial}`, ...JSON.parse(o.body) };
      made.push(row);
      if (options.loseCreate) throw Error(CANARY);
      if (options.driftAfterCreate) {
        live = `projects/${PROJECT}/rulesets/foreign`;
        made.push({
          name: live,
          source: { files: [{ name: 'firestore.rules', content: 'foreign' }] },
        });
      }
      return { status: 200, body: row };
    }
    if (o.method === 'PATCH' || (route === `projects/${PROJECT}/releases` && o.method === 'POST')) {
      live = JSON.parse(o.body).release?.rulesetName ?? JSON.parse(o.body).rulesetName;
      if (options.wrongReadBack)
        made.find((x) => x.name === live).source.files[0].content = 'different';
      return { status: 200, body: { rulesetName: live } };
    }
    assert.fail(`unexpected fixture path ${route}`);
  });
  return { f, made };
}
test('Rules switch is last and exact live source is read back', async () => {
  const { f } = rulesFixture();
  assert.equal(await execute(request('apply-rules', rulesInput()), f), 'done');
  assert.deepEqual(
    mutations(f).map((x) => new URL(x.url).pathname),
    [`/v1/projects/${PROJECT}/rulesets`, `/v1/projects/${PROJECT}/releases`]
  );
  assert.equal(f.calls.at(-1).method, 'GET');
  assert.match(f.calls.at(-1).url, /rulesets\/new1$/);
});
test('a lost Ruleset create is recovered by bounded source lookup before any second create', async () => {
  const first = rulesFixture({ loseCreate: true });
  await failure(() => execute(request('apply-rules', rulesInput()), first.f), 'unknown');
  assert.equal(mutations(first.f).length, 1);
  const next = rulesFixture({ made: first.made });
  assert.equal(await execute(request('apply-rules', rulesInput()), next.f), 'done');
  assert.equal(mutations(next.f).filter((c) => c.url.endsWith('/rulesets')).length, 0);
});
test('visible Rules drift and mismatched read-back never claim completion', async () => {
  const drift = rulesFixture({ driftAfterCreate: true });
  await failure(() => execute(request('apply-rules', rulesInput()), drift.f), 'unknown');
  assert.equal(mutations(drift.f).length, 1);
  const mismatch = rulesFixture({ wrongReadBack: true });
  await failure(() => execute(request('apply-rules', rulesInput()), mismatch.f), 'unknown');
});
test('foreign release needs the exact replaced digest and is otherwise untouched', async () => {
  const name = `projects/${PROJECT}/rulesets/foreign`,
    made = [{ name, source: { files: [{ name: 'firestore.rules', content: 'foreign' }] } }];
  const refused = rulesFixture({ live: name, made });
  await failure(() => execute(request('apply-rules', rulesInput()), refused.f), 'rules-foreign');
  assert.equal(mutations(refused.f).length, 0);
  const accepted = rulesFixture({ live: name, made });
  assert.equal(
    await execute(
      request('apply-rules', rulesInput({ replacedDigest: hash('foreign') })),
      accepted.f
    ),
    'done'
  );
});
test('fixed provider faults do not expose token/body/header text', async () => {
  for (const [status, code] of [
    [403, 'permission-denied'],
    [429, 'quota-exceeded'],
    [503, 'unknown'],
  ]) {
    const f = fixture(() => ({ status, body: { error: { message: CANARY } } }));
    await failure(
      () => execute(request('observe-database', { project: PROJECT, location: 'us-central1' }), f),
      code
    );
  }
});
test('real HTTP fetch never retries a rejected mutation', async () => {
  let posts = 0;
  const server = http.createServer((req, res) => {
    if (req.method === 'POST') posts++;
    res.writeHead(503, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify({ error: { message: CANARY } }));
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  try {
    const result = await fetchJson(
      `http://127.0.0.1:${server.address().port}`,
      { method: 'POST', body: '{}' },
      Date.now() + 3000
    );
    assert.equal(result.status, 503);
    assert.equal(posts, 1);
  } finally {
    await new Promise((resolve) => server.close(resolve));
  }
});
test('a version/layout stub gate runs before credential access and redacts thrown text', async () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'tmt-firebase-stub-'));
  try {
    fs.mkdirSync(path.join(root, 'lib'));
    for (const file of ['auth', 'api', 'apiv2', 'configstore', 'logger'])
      fs.writeFileSync(path.join(root, 'lib', `${file}.js`), 'module.exports={}');
    fs.writeFileSync(
      path.join(root, 'package.json'),
      JSON.stringify({ name: 'firebase-tools', version: '0.0.0' })
    );
    const r = await run(JSON.stringify(request('account')), root, {
      credential: () => {
        throw Error(CANARY);
      },
    });
    assert.deepEqual(r, { version: 1, error: 'unsupported-tool' });
    assert.equal(JSON.stringify(r).includes(CANARY), false);
    fs.writeFileSync(
      path.join(root, 'package.json'),
      JSON.stringify({ name: 'firebase-tools', version: '15.29.0' })
    );
    const thrown = await run(JSON.stringify(request('account')), root, {
      deadline: Date.now() + 3000,
      credential: () => {
        throw Error(CANARY);
      },
    });
    assert.deepEqual(thrown, { version: 1, error: 'unknown' });
    assert.equal(JSON.stringify(thrown).includes(CANARY), false);
  } finally {
    fs.rmSync(root, { recursive: true });
  }
});

test('bounded inventory refuses incomplete pagination before Ruleset creation', async () => {
  const f = fixture((url) => {
    if (url.includes('/releases/')) return { status: 404, body: {} };
    return { status: 200, body: { rulesets: [], nextPageToken: 'same' } };
  });
  await failure(() => execute(request('apply-rules', rulesInput()), f), 'unknown');
  assert.equal(mutations(f).length, 0);
});
test('a denied read-back after a successful mutation is Unknown, not a no-effect refusal', async () => {
  const f = fixture((url, o) => {
    if (url.includes('/releases/') && o.method === 'GET') {
      if (mutations(f).some((c) => c.url.includes('/releases')))
        return { status: 403, body: { error: { message: CANARY } } };
      return { status: 404, body: {} };
    }
    if (url.includes('/rulesets') && o.method === 'GET')
      return { status: 200, body: { rulesets: [] } };
    if (url.endsWith('/rulesets') && o.method === 'POST')
      return { status: 200, body: { name: `projects/${PROJECT}/rulesets/new` } };
    return { status: 200, body: {} };
  });
  await failure(() => execute(request('apply-rules', rulesInput()), f), 'unknown');
  assert.equal(mutations(f).length, 2);
});
test('a direct quota rejection of the first mutation stays a fixed QuotaExceeded fault', async () => {
  const f = fixture((_, o) =>
    o.method === 'GET'
      ? { status: 404, body: {} }
      : { status: 429, body: { error: { message: CANARY } } }
  );
  await failure(
    () => execute(request('apply-database', { project: PROJECT, location: 'us-central1' }), f),
    'quota-exceeded'
  );
  assert.equal(mutations(f).length, 1);
});

test('HTTP body bounds and malformed JSON refuse without exposing response text', async () => {
  const server = http.createServer((req, res) => {
    res.writeHead(200, { 'Content-Type': 'application/json' });
    res.end(req.url === '/oversized' ? 'x'.repeat(4 * 1024 * 1024 + 1) : CANARY);
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  try {
    for (const route of ['oversized', 'malformed']) {
      await assert.rejects(
        () =>
          fetchJson(
            `http://127.0.0.1:${server.address().port}/${route}`,
            { method: 'GET' },
            Date.now() + 3000
          ),
        (error) => !String(error).includes(CANARY)
      );
    }
  } finally {
    await new Promise((resolve) => server.close(resolve));
  }
});
test('an already spent HTTP deadline makes zero requests', async () => {
  await failure(
    () => fetchJson('http://127.0.0.1:1', { method: 'POST' }, Date.now() - 1),
    'unknown'
  );
});

test('login is re-resolved immediately before each effect, not cached from observation', async () => {
  let selected = ACCOUNT,
    checks = 0;
  const f = fixture(() => ({ status: 404, body: {} }));
  f.credential = async () => {
    selected = ++checks === 1 ? ACCOUNT : 'changed@example.test';
    return { account: selected, token: CANARY };
  };
  const http = f.http;
  f.http = async (url, options) =>
    url.endsWith('/oauth2/v3/userinfo')
      ? { status: 200, body: { email: selected, email_verified: true } }
      : http(url, options);
  await failure(
    () => execute(request('apply-database', { project: PROJECT, location: 'us-central1' }), f),
    'account-changed'
  );
  assert.equal(checks, 2);
  assert.equal(mutations(f).length, 0);
});

// Remote-owned, in-memory API lifecycle fixture; no provider/account acceptance.
function hostedFixture() {
  const files = [
    {
      path: '/index.html',
      gzipDigest: hash(Buffer.from('gzip-fixture')),
      rawDigest: hash('raw'),
      rawLength: 3,
      gzipLength: 12,
      contentType: 'text/html',
    },
  ];
  const state = { apps: [], site: null, versions: [], files: [], live: null, source, fail: null };
  const calls = [];
  let clock = Date.now();
  const f = {
    deadline: clock + 30000,
    now: () => clock,
    wait: async (ms) => {
      clock += ms;
    },
    credential: async () => ({ account: ACCOUNT, token: CANARY }),
    http: async (url, options) => {
      const { method } = options;
      const payload = Buffer.isBuffer(options.body)
        ? options.body
        : options.body
          ? JSON.parse(options.body)
          : null;
      calls.push({ url, method, payload });
      if (url.endsWith('/oauth2/v3/userinfo'))
        return { status: 200, body: { email: ACCOUNT, email_verified: true } };
      if (state.fail?.(url, options)) throw Error(CANARY);
      const ok = (body) => ({ status: 200, body }),
        missing = () => ({ status: 404, body: {} });
      const route = new URL(url).pathname;
      if (route === `/v1beta1/projects/${PROJECT}/webApps`) {
        if (method === 'GET') return ok({ apps: state.apps });
        if (!state.pendingApp)
          state.apps.push({
            appId: '1:123:web:mine',
            name: `projects/${PROJECT}/webApps/1:123:web:mine`,
            projectId: PROJECT,
            state: 'ACTIVE',
            displayName: payload.displayName,
          });
        return ok({ name: 'operations/app-one', done: false });
      }
      if (route === '/v1beta1/operations/app-one')
        return state.pendingApp ? ok({ done: false }) : missing();
      if (route.endsWith('/config'))
        return ok({
          apiKey: 'public-key',
          authDomain: `${PROJECT}.firebaseapp.com`,
          projectId: PROJECT,
          appId: '1:123:web:mine',
          extra: 'never disclosed',
        });
      if (route === `/v1beta1/projects/${PROJECT}/sites/${PROJECT}`) {
        if (method === 'PATCH') {
          assert.equal(new URL(url).searchParams.get('updateMask'), 'appId');
          assert.deepEqual(Object.keys(payload), ['appId']);
          assert.ok(!state.site.appId);
          state.site.appId = payload.appId;
        }
        return state.site ? ok(state.site) : missing();
      }
      if (route === `/v1beta1/projects/${PROJECT}/sites` && method === 'POST') {
        assert.equal(new URL(url).searchParams.get('siteId'), PROJECT);
        assert.deepEqual(payload, {}); // Association has one separate plan-listed step.
        state.site = {
          name: `projects/${PROJECT}/sites/${PROJECT}`,
          defaultUrl: `https://${PROJECT}.web.app`,
        };
        return ok(state.site);
      }
      if (route === `/v1beta1/sites/${PROJECT}/versions`) {
        if (method === 'GET') return ok({ versions: state.versions });
        const version = {
          name: `sites/${PROJECT}/versions/one`,
          createTime: new Date().toISOString(),
          status: 'CREATED',
          ...payload,
        };
        state.versions.push(version);
        return ok(version);
      }
      if (route === `/v1beta1/sites/${PROJECT}/versions/one`) {
        if (method === 'PATCH') state.versions[0].status = payload.status;
        return ok(state.versions[0]);
      }
      if (route.endsWith('/versions/one/files')) return ok({ files: state.files });
      if (route.endsWith('/versions/one:populateFiles')) {
        state.files = Object.entries(payload.files).map(([path, hash]) => ({
          path,
          hash,
          status: 'EXPECTED',
        }));
        return ok({
          uploadRequiredHashes: state.files.map((f) => f.hash),
          uploadUrl:
            state.uploadUrl ??
            `https://upload-firebasehosting.googleapis.com/upload/sites/${PROJECT}/versions/one/files`,
        });
      }
      if (url.startsWith('https://upload-firebasehosting.googleapis.com/')) {
        assert.equal(payload.toString(), 'gzip-fixture');
        state.files.forEach((f) => (f.status = 'ACTIVE'));
        return ok(null);
      }
      if (route === `/v1beta1/sites/${PROJECT}/channels/live`)
        return ok(state.live ? { release: state.live } : {});
      if (route === `/v1beta1/sites/${PROJECT}/releases`) {
        assert.equal(new URL(url).searchParams.get('versionName'), state.versions[0].name);
        state.live = { name: `sites/${PROJECT}/releases/one`, version: state.versions[0] };
        return ok(state.live);
      }
      if (route === `/v1/${release}`)
        return ok({ rulesetName: `projects/${PROJECT}/rulesets/current` });
      if (route === `/v1/projects/${PROJECT}/rulesets/current`)
        return ok({ source: { files: [{ name: 'firestore.rules', content: state.source }] } });
      assert.fail(`unexpected fixture route ${method} ${url}`);
    },
  };
  const input = {
    project: PROJECT,
    deployment: ID,
    planDigest: 'a'.repeat(64),
    hosting: {
      site: PROJECT,
      publicUrl: `https://${PROJECT}.web.app`,
      createSite: true,
      configureSite: true,
      siteAppId: null,
      createWebApp: true,
      webApp: null,
      content: { version: 1, files, config: { headers: [], rewrites: [] }, digest: 'b'.repeat(64) },
      replaces: 'none',
      replacedFingerprint: null,
    },
    checkpoint: {
      envelope: { fixture: true },
      appId: null,
      operation: null,
      version: null,
      versionCreatedMs: null,
      publication: null,
    },
    steps: [],
    action: 'create',
    hash: null,
    bytesBase64: null,
    source,
  };
  async function call(operation, action, extra = {}) {
    const r = await execute(
      { ...request(operation), account: ACCOUNT, input: { ...input, action, ...extra } },
      f
    );
    if (r.checkpoint) input.checkpoint = r.checkpoint;
    return r;
  }
  return { f, state, calls, input, call };
}
test('Hosting associates the selected app before staging and verifies that same association', async () => {
  const { state, calls, call } = hostedFixture();
  assert.equal((await call('observe-hosting', 'web-app')).state, 'absent');
  assert.equal((await call('apply-hosting', 'web-app')).state, 'done');
  assert.equal((await call('observe-hosting', 'web-app')).state, 'satisfied');
  await call('apply-hosting', 'site');
  await call('apply-hosting', 'configure');
  await call('apply-hosting', 'create');
  assert.equal(state.versions[0].labels['tmt-content'].length, 32);
  assert.ok(Object.values(state.versions[0].labels).every((value) => value.length <= 63));
  await call('apply-hosting', 'populate');
  await call('apply-hosting', 'upload', {
    hash: hash('gzip-fixture'),
    bytesBase64: Buffer.from('gzip-fixture').toString('base64'),
  });
  assert.equal(state.live, null);
  await call('apply-hosting', 'finalize');
  await call('apply-hosting', 'release');
  const result = await call('verify-hosting', 'verify');
  assert.equal(result.siteAppId, state.site.appId);
  state.site.appId = 'different-app';
  await failure(() => call('verify-hosting', 'verify'), 'unknown');
  state.site.appId = result.siteAppId;
  assert.deepEqual(result.publicConfig, {
    apiKey: 'public-key',
    authDomain: `${PROJECT}.firebaseapp.com`,
    projectId: PROJECT,
    appId: '1:123:web:mine',
  });
  assert.equal(result.release, `sites/${PROJECT}/releases/one`);
  assert.equal(calls.filter((c) => c.method === 'POST' && c.url.includes('/releases?')).length, 1);
  assert.equal((await call('observe-hosting', 'release')).state, 'satisfied');
  assert.equal(calls.filter((c) => c.method === 'POST' && c.url.includes('/releases?')).length, 1);
});
test('ambiguous stage creation reconciles the same version; wrong files and expired stage never publish', async () => {
  const x = hostedFixture();
  await x.call('apply-hosting', 'web-app');
  await x.call('observe-hosting', 'web-app');
  await x.call('apply-hosting', 'site');
  await x.call('apply-hosting', 'configure');
  await x.call('apply-hosting', 'create');
  x.input.checkpoint.version = null;
  x.input.checkpoint.versionCreatedMs = null;
  assert.equal((await x.call('observe-hosting', 'create')).state, 'satisfied');
  assert.equal(x.calls.filter((c) => c.method === 'POST' && c.url.endsWith('/versions')).length, 1);
  x.state.versions[0].createTime = new Date(Date.now() - 13 * 60 * 60 * 1000).toISOString();
  x.input.checkpoint.versionCreatedMs = Date.now() - 13 * 60 * 60 * 1000;
  await failure(() => x.call('observe-hosting', 'create'), 'unknown');
  assert.equal(x.state.live, null);
});
test('upload faults, cross-host URL, and joint read-back mismatch stay unknown without publication', async () => {
  const x = hostedFixture();
  await x.call('apply-hosting', 'web-app');
  await x.call('observe-hosting', 'web-app');
  await x.call('apply-hosting', 'site');
  await x.call('apply-hosting', 'configure');
  await x.call('apply-hosting', 'create');
  await x.call('apply-hosting', 'populate');
  x.state.fail = (url) => url.startsWith('https://upload-');
  await failure(
    () =>
      x.call('apply-hosting', 'upload', {
        hash: hash('gzip-fixture'),
        bytesBase64: Buffer.from('gzip-fixture').toString('base64'),
      }),
    'unknown'
  );
  assert.equal(x.state.live, null);
  x.state.fail = null;
  await x.call('apply-hosting', 'upload', {
    hash: hash('gzip-fixture'),
    bytesBase64: Buffer.from('gzip-fixture').toString('base64'),
  });
  await x.call('apply-hosting', 'finalize');
  await x.call('apply-hosting', 'release');
  x.state.source = 'different';
  await failure(() => x.call('verify-hosting', 'verify'), 'unknown');
  assert.equal(x.input.checkpoint.publication, null);
});

test('provider-directed upload URL cannot send a credential to another host', async () => {
  const x = hostedFixture();
  await x.call('apply-hosting', 'web-app');
  await x.call('observe-hosting', 'web-app');
  await x.call('apply-hosting', 'site');
  await x.call('apply-hosting', 'configure');
  await x.call('apply-hosting', 'create');
  x.state.uploadUrl = 'https://evil.invalid/upload';
  await failure(() => x.call('apply-hosting', 'populate'), 'unknown');
  assert.equal(
    x.calls.some((c) => c.url.startsWith('https://evil.invalid')),
    false
  );
  assert.equal(x.state.live, null);
});

test('pending web app is observed within a separate bounded clock budget before Building', async () => {
  const x = hostedFixture();
  x.state.pendingApp = true;
  const start = x.f.now();
  const result = await x.call('apply-hosting', 'web-app');
  assert.equal(result.state, 'building');
  assert.equal(result.checkpoint.operation, 'operations/app-one');
  assert.equal(x.f.now() - start, 15000);
  assert.equal(x.calls.filter((c) => c.method === 'POST' && c.url.endsWith('/webApps')).length, 1);
  assert.equal(x.state.site, null);
  x.state.pendingApp = false;
  x.state.apps.push({
    name: `projects/${PROJECT}/webApps/1:123:web:mine`,
    appId: '1:123:web:mine',
    projectId: PROJECT,
    displayName: `tmt Remote (${ID})`,
    state: 'ACTIVE',
  });
  assert.equal((await x.call('observe-hosting', 'web-app')).state, 'satisfied');
  assert.equal(x.calls.filter((c) => c.method === 'POST' && c.url.endsWith('/webApps')).length, 1);
});

test('existing unassociated site is configured once and nonempty drift never repoints it', async () => {
  const x = hostedFixture();
  await x.call('apply-hosting', 'web-app');
  x.state.site = {
    name: `projects/${PROJECT}/sites/${PROJECT}`,
    defaultUrl: `https://${PROJECT}.web.app`,
  };
  assert.equal((await x.call('observe-hosting', 'configure')).state, 'absent');
  assert.equal((await x.call('apply-hosting', 'configure')).state, 'done');
  assert.equal((await x.call('apply-hosting', 'configure')).state, 'done');
  assert.equal(x.state.site.appId, x.input.checkpoint.appId);
  const mutationsBefore = x.calls.filter((c) => c.method === 'PATCH').length;
  assert.equal(mutationsBefore, 1);
  x.state.site.appId = 'foreign-app';
  await failure(() => x.call('apply-hosting', 'configure'), 'unknown');
  assert.equal(x.calls.filter((c) => c.method === 'PATCH').length, mutationsBefore);
  assert.equal(x.state.site.appId, 'foreign-app');
});
