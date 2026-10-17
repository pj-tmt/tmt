'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const http = require('node:http');
const crypto = require('node:crypto');
const {
  execute,
  validate,
  run,
  fetchJson,
  providerOperation,
} = require('../../src/deploy_firestore/helper.cjs');
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
      const parsed = new URL(url);
      if (
        options.method === 'GET' &&
        parsed.origin === 'https://firestore.googleapis.com' &&
        /^\/v1\/projects\/[^/]+\/databases\/[^/]+\/collectionGroups\/[^/]+\/(fields|indexes)$/.test(
          parsed.pathname
        )
      ) {
        const token = parsed.searchParams.get('pageToken');
        const filter = parsed.pathname.endsWith('/fields')
          ? '?filter=indexConfig.usesAncestorConfig=false%20OR%20ttlConfig:*'
          : '';
        const expected =
          filter + (token ? `${filter ? '&' : '?'}pageToken=${encodeURIComponent(token)}` : '');
        if (parsed.search !== expected)
          return { status: 400, body: { error: { code: 400, status: 'INVALID_ARGUMENT' } } };
      }
      if (
        options.method === 'GET' &&
        parsed.origin === 'https://firebaserules.googleapis.com' &&
        /^\/v1\/projects\/[^/]+\/rulesets$/.test(parsed.pathname) &&
        Number(parsed.searchParams.get('pageSize')) > 100
      )
        return { status: 400, body: { error: { code: 400, status: 'INVALID_ARGUMENT' } } };
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
test('fields fake pins the proven request and refuses pageSize or whole-filter encoding', async () => {
  let accepted = 0;
  const f = fixture(() => {
    accepted++;
    return { status: 200, body: { fields: [] } };
  });
  const base = `https://firestore.googleapis.com/v1/${database}/collectionGroups/-/fields`;
  const proven = '?filter=indexConfig.usesAncestorConfig=false%20OR%20ttlConfig:*';
  for (const query of [
    '',
    `${proven}&pageSize=200`,
    `?filter=${encodeURIComponent('indexConfig.usesAncestorConfig=false OR ttlConfig:*')}`,
    `?filter=${encodeURIComponent('indexConfig.usesAncestorConfig=false OR ttlConfig:*')}&pageSize=200`,
    `?filter=${encodeURIComponent('indexConfig.usesAncestorConfig:false OR ttlConfig:*')}&pageSize=200`,
    `${proven}&unknown=true`,
  ])
    assert.equal((await f.http(base + query, { method: 'GET' })).status, 400);
  assert.equal(accepted, 0);
  for (const suffix of ['', '&pageToken=next%20page%26token'])
    assert.equal((await f.http(base + proven + suffix, { method: 'GET' })).status, 200);
  assert.equal(accepted, 2);
});
test('budget returns database presence from its existing inventory read only', async () => {
  for (const present of [false, true]) {
    const f = fixture((url) => {
      if (url.endsWith('/(default)'))
        return { status: present ? 200 : 404, body: present ? { name: database } : {} };
      if (url.includes('/-/fields')) return { status: 200, body: { fields: [] } };
      if (url.includes('/-/indexes')) return { status: 200, body: { indexes: [] } };
      assert.fail(url);
    });
    assert.deepEqual(
      await execute(request('index-budget', { project: PROJECT, fields: [], composites: [] }), f),
      { status: 'within-budget', databasePresent: present }
    );
    assert.equal(f.calls.filter((call) => call.url.endsWith('/(default)')).length, 1);
    assert.equal(mutations(f).length, 0);
  }
});
for (const operation of ['index-budget', 'apply-index']) {
  test(`${operation} sends the pinned fields URL on every bounded page`, async () => {
    let pages = 0;
    const f = fixture((url, options) => ({
      status: 200,
      body:
        options.method !== 'GET'
          ? {}
          : url.includes('/-/fields')
            ? { fields: [], ...(pages++ === 0 ? { nextPageToken: 'next page&token' } : {}) }
            : url.includes('/-/indexes')
              ? { indexes: [] }
              : url.endsWith('/fields/sequence')
                ? { name: field, indexConfig: { indexes: [] } }
                : {},
    }));
    assert.deepEqual(
      await execute(
        request(
          operation,
          operation === 'index-budget'
            ? { project: PROJECT, fields: [], composites: [] }
            : indexInput
        ),
        f
      ),
      operation === 'index-budget' ? { status: 'within-budget', databasePresent: true } : 'done'
    );
    const listed = f.calls.filter((call) => call.url.includes('/-/fields'));
    assert.equal(listed.length, 2);
    for (const [index, call] of listed.entries()) {
      const params = new URL(call.url).searchParams;
      assert.equal(call.method, 'GET');
      assert.equal(params.get('filter'), 'indexConfig.usesAncestorConfig=false OR ttlConfig:*');
      assert.equal(params.get('pageSize'), null);
      assert.equal(params.get('pageToken'), index === 0 ? null : 'next page&token');
      assert.deepEqual(
        [...params.keys()].sort(),
        index === 0 ? ['filter'] : ['filter', 'pageToken']
      );
      assert.equal(
        call.url,
        `https://firestore.googleapis.com/v1/${database}/collectionGroups/-/fields?filter=indexConfig.usesAncestorConfig=false%20OR%20ttlConfig:*${index === 0 ? '' : '&pageToken=next%20page%26token'}`
      );
    }
    assert.equal(mutations(f).length, operation === 'index-budget' ? 0 : 1);
  });
}
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

test('Rules inventory stays within the documented 100-row page maximum', async () => {
  const name = `projects/${PROJECT}/rulesets/existing`;
  let listed = 0;
  const f = fixture((url, options) => {
    if (url.includes('/rulesets?'))
      return {
        status: 200,
        body:
          listed++ === 0
            ? { rulesets: [], nextPageToken: 'next rules&page' }
            : { rulesets: [{ name }] },
      };
    if (url.endsWith('/rulesets/existing'))
      return {
        status: 200,
        body: { source: { files: [{ name: 'firestore.rules', content: source }] } },
      };
    if (url.endsWith('/releases') && options.method === 'POST')
      return { status: 200, body: JSON.parse(options.body) };
    if (url.includes('/releases/'))
      return mutations(f).length === 0
        ? { status: 404, body: {} }
        : { status: 200, body: { rulesetName: name } };
    assert.fail(url);
  });
  const oversized = `https://firebaserules.googleapis.com/v1/projects/${PROJECT}/rulesets?pageSize=200`;
  assert.equal((await f.http(oversized, { method: 'GET' })).status, 400);
  assert.equal(listed, 0);
  f.calls.length = 0;
  assert.equal(await execute(request('apply-rules', rulesInput()), f), 'done');
  const pages = f.calls.filter((call) => call.url.includes('/rulesets?'));
  assert.equal(pages.length, 2);
  assert.deepEqual(
    pages.map((call) => Object.fromEntries(new URL(call.url).searchParams)),
    [{ pageSize: '100' }, { pageSize: '100', pageToken: 'next rules&page' }]
  );
  assert.equal(mutations(f).length, 1);
  assert.equal(mutations(f)[0].url.endsWith('/releases'), true);
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
      if (route.endsWith('/versions/one/files')) {
        // The default is the remaining upload inventory, not every Version file.
        const status = new URL(url).searchParams.get('status') ?? 'EXPECTED';
        assert.ok(['EXPECTED', 'ACTIVE'].includes(status));
        return ok({ files: state.files.filter((file) => file.status === status) });
      }
      if (route.endsWith('/versions/one:populateFiles')) {
        state.files = Object.entries(payload.files).map(([path, hash]) => ({
          path,
          hash,
          status: state.activeHashes?.includes(hash) ? 'ACTIVE' : 'EXPECTED',
        }));
        return ok({
          ...(state.omitRequired
            ? {}
            : { uploadRequiredHashes: state.files.filter((f) => f.status === 'EXPECTED').map((f) => f.hash) }),
          uploadUrl:
            state.uploadUrl ??
            `https://upload-firebasehosting.googleapis.com/upload/sites/${PROJECT}/versions/one/files`,
        });
      }
      if (url.startsWith('https://upload-firebasehosting.googleapis.com/')) {
        assert.equal(payload.toString(), 'gzip-fixture');
        state.files
          .filter((f) => url.endsWith('/' + f.hash))
          .forEach((f) => (f.status = 'ACTIVE'));
        return ok(null);
      }
      if (route === `/v1beta1/sites/${PROJECT}/channels/live`)
        return ok(state.live ? { release: {
          ...state.live,
          ...(Object.hasOwn(state, 'channelReleaseName') ? { name: state.channelReleaseName } : {}),
        } } : {});
      if (route === `/v1beta1/sites/${PROJECT}/releases`) {
        assert.equal(new URL(url).searchParams.get('versionName'), state.versions[0].name);
        state.live = { name: `sites/${PROJECT}/releases/one`, version: state.versions[0] };
        return ok({
          ...state.live,
          ...(Object.hasOwn(state, 'postReleaseName') ? { name: state.postReleaseName } : {}),
        });
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
    contentDigest: 'c'.repeat(64),
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
test('Hosting inventory admits an omitted default WebApp display name without an effect', async () => {
  const x = hostedFixture();
  x.state.apps.push({
    name: `projects/${PROJECT}/webApps/1:123:web:other`,
    appId: '1:123:web:other',
    projectId: PROJECT,
    state: 'ACTIVE',
  });
  const observed = await execute(request('hosting-inventory', { project: PROJECT }), x.f);
  assert.deepEqual(observed.webApps, [{ id: '1:123:web:other', displayName: '', state: 'ACTIVE' }]);
  assert.equal((await x.call('observe-hosting', 'web-app')).state, 'absent');
  assert.equal(x.calls.filter((c) => c.method !== 'GET').length, 0);
});

test('Hosting app adoption ignores an unrelated unnamed app and retains the exact selected identity', async () => {
  const x = hostedFixture();
  await x.call('apply-hosting', 'web-app');
  x.state.apps.unshift({
    name: `projects/${PROJECT}/webApps/1:123:web:other`,
    appId: '1:123:web:other',
    projectId: PROJECT,
    state: 'ACTIVE',
  });
  const before = x.calls.filter((c) => c.method !== 'GET').length;
  assert.equal((await x.call('apply-hosting', 'web-app')).state, 'done');
  assert.equal(x.input.checkpoint.appId, '1:123:web:mine');
  assert.equal(x.calls.filter((c) => c.method !== 'GET').length, before);
});

test('Hosting omitted-name admission still refuses malformed or foreign WebApps without effects', async () => {
  const app = {
    name: `projects/${PROJECT}/webApps/1:123:web:other`,
    appId: '1:123:web:other',
    projectId: PROJECT,
    state: 'ACTIVE',
  };
  for (const value of [
    ...[null, 0, false, [], {}].map((displayName) => ({ ...app, displayName })),
    { ...app, projectId: 'other-project' },
    { ...app, appId: null },
    { ...app, name: `projects/other-project/webApps/1:123:web:other` },
    { ...app, state: null },
    null,
    [],
  ]) {
    const x = hostedFixture();
    x.state.apps.push(value);
    await failure(() => execute(request('hosting-inventory', { project: PROJECT }), x.f), 'unknown');
    assert.equal(x.calls.filter((c) => c.method !== 'GET').length, 0);
  }
});

async function finalizedHostingFixture() {
  const x = hostedFixture();
  for (const action of ['web-app', 'site', 'configure', 'create', 'populate'])
    await x.call('apply-hosting', action);
  await x.call('apply-hosting', 'upload', {
    hash: hash('gzip-fixture'),
    bytesBase64: Buffer.from('gzip-fixture').toString('base64'),
  });
  await x.call('apply-hosting', 'finalize');
  return x;
}

test('Hosting admits either same-site live release name without rewriting it', async () => {
  const names = [`sites/${PROJECT}/releases/one`, `sites/${PROJECT}/channels/live/releases/one`];
  for (const post of names) for (const channel of names) {
    const x = await finalizedHostingFixture();
    x.state.postReleaseName = post;
    x.state.channelReleaseName = channel;
    assert.equal((await x.call('apply-hosting', 'release')).state, 'done');
    assert.equal((await x.call('verify-hosting', 'verify')).release, channel);
    assert.equal((await x.call('observe-hosting', 'release')).state, 'satisfied');
    assert.equal(x.calls.filter((c) => c.method === 'POST' && c.url.includes('/releases?')).length, 1);
  }
});

test('Hosting refuses hostile release answers and mismatched live release identities without a second POST', async () => {
  const hostile = [
    `sites/other-site/releases/one`, `sites/${PROJECT}/channels/preview/releases/one`,
    `sites/${PROJECT}/releases/`, `sites/${PROJECT}/channels/live/releases/`,
    `sites/${PROJECT}/releases/one/extra`, `sites/${PROJECT}/channels/live/releases/one/extra`,
    `sites/${PROJECT}/releases/one?extra`, `sites/${PROJECT}/releases/${'a'.repeat(257)}`, null, undefined,
  ];
  for (const field of ['postReleaseName', 'channelReleaseName']) for (const name of hostile) {
    const x = await finalizedHostingFixture();
    x.state[field] = name;
    await failure(() => x.call('apply-hosting', 'release'), 'unknown');
    assert.equal(x.calls.filter((c) => c.method === 'POST' && c.url.includes('/releases?')).length, 1);
  }
  const x = await finalizedHostingFixture();
  x.state.postReleaseName = `sites/${PROJECT}/releases/one`;
  x.state.channelReleaseName = `sites/${PROJECT}/channels/live/releases/two`;
  await failure(() => x.call('apply-hosting', 'release'), 'unknown');
  assert.equal(x.calls.filter((c) => c.method === 'POST' && c.url.includes('/releases?')).length, 1);
});

test('Hosting verification compares release identity across aliases and refuses changed IDs', async () => {
  const alias = await finalizedHostingFixture();
  await alias.call('apply-hosting', 'release');
  const names = [`sites/${PROJECT}/releases/one`, `sites/${PROJECT}/channels/live/releases/one`];
  let aliasReads = 0;
  Object.defineProperty(alias.state, 'channelReleaseName', { get() {
    return names[aliasReads++ % names.length];
  } });
  assert.ok(names.includes((await alias.call('verify-hosting', 'verify')).release));
  const x = await finalizedHostingFixture();
  await x.call('apply-hosting', 'release');
  let reads = 0;
  Object.defineProperty(x.state, 'channelReleaseName', { get() {
    return ++reads === 1 ? `sites/${PROJECT}/releases/one`
      : `sites/${PROJECT}/channels/live/releases/two`;
  } });
  await failure(() => x.call('verify-hosting', 'verify'), 'unknown');
  assert.equal(x.calls.filter((c) => c.method === 'POST' && c.url.includes('/releases?')).length, 1);
});

test('Hosting requires the supplied Rust content digest before any provider read', async () => {
  const valid = hostedFixture();
  assert.equal((await valid.call('observe-hosting', 'web-app')).state, 'absent');
  for (const digest of [undefined, null, ['c'.repeat(64)], '', 'a'.repeat(63), 'A'.repeat(64), 'g'.repeat(64)]) {
    const f = hostedFixture();
    if (digest === undefined) delete f.input.contentDigest;
    else f.input.contentDigest = digest;
    await failure(() => f.call('observe-hosting', 'web-app'), 'provider-rejected');
    assert.equal(f.calls.length, 0);
  }
});
test('Hosting associates the selected app before staging and verifies that same association', async () => {
  const { state, calls, call, input } = hostedFixture();
  assert.equal((await call('observe-hosting', 'web-app')).state, 'absent');
  assert.equal((await call('apply-hosting', 'web-app')).state, 'done');
  assert.equal((await call('observe-hosting', 'web-app')).state, 'satisfied');
  await call('apply-hosting', 'site');
  await call('apply-hosting', 'configure');
  await call('apply-hosting', 'create');
  assert.equal(state.versions[0].labels['tmt-content'], input.contentDigest.slice(0, 32));
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
  // The reply is refused, but exact read-only inventory can confirm the effect.
  assert.equal((await x.call('apply-hosting', 'populate')).state, 'done');
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
const composite = {
  collectionGroup: 'log',
  queryScope: 'COLLECTION',
  fields: [
    { fieldPath: 'room', order: 'ASCENDING' },
    { fieldPath: 'writer', order: 'ASCENDING' },
    { fieldPath: 'position', order: 'ASCENDING' },
  ],
};
const compositeInput = { project: PROJECT, index: composite };
function compositeFixture(indexes = [], lose = false) {
  const made = [...indexes];
  const f = fixture((url, options) => {
    if (new URL(url).pathname.endsWith('/collectionGroups/-/indexes'))
      return { status: 200, body: { indexes: made } };
    if (options.method === 'POST') {
      made.push({
        name: `${database}/collectionGroups/log/indexes/new`,
        state: 'CREATING',
        ...JSON.parse(options.body),
      });
      if (lose) throw Error(CANARY);
      return { status: 200, body: { name: `${database}/operations/new` } };
    }
    assert.fail(`unexpected route ${url}`);
  });
  return { f, made };
}
test('composite inventory keeps the pinned first URL and bounded token-only continuation', async () => {
  let listed = 0;
  const base = `https://firestore.googleapis.com/v1/${database}/collectionGroups/-/indexes`;
  const f = fixture(() => ({
    status: 200,
    body: {
      indexes: [],
      ...(listed++ === 0 ? { nextPageToken: 'next index&page' } : {}),
    },
  }));
  assert.equal((await f.http(`${base}?pageSize=200`, { method: 'GET' })).status, 400);
  assert.equal(listed, 0);
  f.calls.length = 0;
  assert.equal(await execute(request('observe-composite', compositeInput), f), 'absent');
  assert.deepEqual(
    f.calls.filter((call) => call.url.includes('/-/indexes')).map((call) => call.url),
    [base, `${base}?pageToken=next%20index%26page`]
  );
  assert.equal(mutations(f).length, 0);
});
test('composite create stays building; retry reconciles exact state without another create', async () => {
  const { f, made } = compositeFixture();
  assert.equal(await execute(request('apply-composite', compositeInput), f), 'building');
  assert.equal(await execute(request('observe-composite', compositeInput), f), 'building');
  made[0].state = 'READY';
  made[0].fields.push({ fieldPath: '__name__', order: 'ASCENDING' });
  assert.equal(await execute(request('observe-composite', compositeInput), f), 'adopted');
  assert.equal(mutations(f).length, 1);
  assert.deepEqual(mutations(f)[0].body, {
    queryScope: composite.queryScope,
    fields: composite.fields,
  });
});
test('lost create reply is unknown and exact-spec observation prevents duplicate allocation', async () => {
  const { f } = compositeFixture([], true);
  await failure(() => execute(request('apply-composite', compositeInput), f), 'unknown');
  assert.equal(await execute(request('apply-composite', compositeInput), f), 'building');
  assert.equal(mutations(f).length, 1);
});
test('repair or malformed composite evidence never becomes ready or gets deleted', async () => {
  for (const state of ['NEEDS_REPAIR', 'STATE_UNSPECIFIED', 'not-a-state']) {
    const { f } = compositeFixture([
      { ...composite, name: `${database}/collectionGroups/log/indexes/one`, state },
    ]);
    await failure(
      () => execute(request('apply-composite', compositeInput), f),
      state === 'NEEDS_REPAIR' ? 'provider-rejected' : 'unknown'
    );
    assert.equal(mutations(f).length, 0);
  }
});
test('composite ceiling is separate and includes unrelated live indexes', async () => {
  for (const count of [199, 200]) {
    const { f } = compositeFixture(
      Array.from({ length: count }, (_, n) => ({
        ...composite,
        collectionGroup: 'other',
        name: `${database}/collectionGroups/other/indexes/i${n}`,
        fields: [
          { fieldPath: `field${n}`, order: 'ASCENDING' },
          { fieldPath: '__name__', order: 'ASCENDING' },
        ],
        state: 'READY',
      }))
    );
    if (count === 199)
      assert.equal(await execute(request('apply-composite', compositeInput), f), 'building');
    else
      await failure(() => execute(request('apply-composite', compositeInput), f), 'quota-exceeded');
    assert.equal(mutations(f).length, count === 199 ? 1 : 0);
  }
});

test('plan-only composite budget counts only missing specs and never mutates', async () => {
  const rows = Array.from({ length: 200 }, (_, n) => ({
    ...composite,
    name: `${database}/collectionGroups/log/indexes/i${n}`,
    fields: [
      { fieldPath: `f${n}`, order: 'ASCENDING' },
      { fieldPath: '__name__', order: 'ASCENDING' },
    ],
    state: 'CREATING',
  }));
  rows[0] = { ...composite, name: rows[0].name, state: 'READY' };
  const f = fixture((url) => ({
    status: 200,
    body: url.endsWith('/(default)')
      ? { name: database }
      : url.includes('/-/fields')
        ? { fields: [] }
        : url.includes('/-/indexes')
          ? { indexes: rows }
          : assert.fail(url),
  }));
  assert.deepEqual(
    await execute(
      request('index-budget', { project: PROJECT, fields: [], composites: [composite] }),
      f
    ),
    { status: 'within-budget', databasePresent: true }
  );
  await failure(
    () =>
      execute(
        request('index-budget', {
          project: PROJECT,
          fields: [],
          composites: [{ ...composite, collectionGroup: 'missing' }],
        }),
        f
      ),
    'quota-exceeded'
  );
  assert.equal(mutations(f).length, 0);
});
test('foreign array indexes count without changing or adopting their configuration', async () => {
  const foreign = {
    name: `${database}/collectionGroups/log/indexes/foreign`,
    queryScope: 'COLLECTION',
    state: 'READY',
    fields: [
      { fieldPath: 'tags.values', arrayConfig: 'CONTAINS' },
      { fieldPath: '__name__', order: 'ASCENDING' },
    ],
  };
  const { f, made } = compositeFixture([foreign]);
  assert.equal(await execute(request('apply-composite', compositeInput), f), 'building');
  assert.deepEqual(made[0], foreign);
  assert.equal(mutations(f).length, 1);
});
test('incomplete composite inventory never allocates and invalid input never asks for credentials', async () => {
  const f = fixture(() => ({ status: 200, body: { indexes: [], nextPageToken: 'repeat' } }));
  await failure(() => execute(request('apply-composite', compositeInput), f), 'unknown');
  assert.equal(mutations(f).length, 0);
  const bad = {
    ...composite,
    fields: [...composite.fields, { fieldPath: 'room', order: 'ASCENDING' }],
  };
  let credentials = 0;
  await failure(
    () =>
      execute(request('apply-composite', { project: PROJECT, index: bad }), {
        credential: async () => {
          credentials++;
          throw Error('never');
        },
      }),
    'provider-rejected'
  );
  assert.equal(credentials, 0);
});

test('preflight HTTP diagnostics name only the closed operation and numeric status', async () => {
  const root = path.join(__dirname, '../fixtures/deploy_firestore/firebase-tools');
  for (const [status, code] of [
    [400, 'provider-rejected'],
    [404, 'provider-rejected'],
    [412, 'provider-rejected'],
    [403, 'permission-denied'],
    [429, 'quota-exceeded'],
    [408, 'unknown'],
    [409, 'unknown'],
  ]) {
    const f = fixture((url) =>
      url.endsWith('/(default)')
        ? { status: 200, body: {} }
        : { status, body: { error: { message: CANARY } } }
    );
    const result = await run(
      JSON.stringify(request('index-budget', { project: PROJECT, fields: [], composites: [] })),
      root,
      f
    );
    assert.deepEqual(result, {
      version: 1,
      error: code,
      provider: { operation: 'fields', status },
    });
    assert.equal(JSON.stringify(result).includes(CANARY), false);
    assert.equal(JSON.stringify(result).includes(PROJECT), false);
    assert.equal(mutations(f).length, 0);
    assert.equal(f.calls.filter((c) => c.url.includes('/-/fields')).length, 1);
  }
});
test('absent database skips inventories, and missing release stays accepted', async () => {
  const f = fixture(() => ({ status: 404, body: {} }));
  assert.deepEqual(
    await execute(request('index-budget', { project: PROJECT, fields: [], composites: [] }), f),
    { status: 'within-budget', databasePresent: false }
  );
  assert.equal(
    f.calls.some((c) => c.url.includes('/collectionGroups/')),
    false
  );
  assert.equal(await execute(request('live-rules', { project: PROJECT }), f), null);
  assert.equal(mutations(f).length, 0);
});
test('missing referenced ruleset and composite inventory retain their own HTTP diagnostics', async () => {
  const root = path.join(__dirname, '../fixtures/deploy_firestore/firebase-tools');
  const f = fixture((url) =>
    url.includes('/releases/')
      ? { status: 200, body: { rulesetName: `projects/${PROJECT}/rulesets/one` } }
      : { status: 404, body: { error: { message: CANARY } } }
  );
  assert.deepEqual(
    await run(JSON.stringify(request('live-rules', { project: PROJECT })), root, f),
    { version: 1, error: 'provider-rejected', provider: { operation: 'rules-set', status: 404 } }
  );
  const indexes = fixture((url) =>
    url.includes('/-/indexes')
      ? { status: 404, body: {} }
      : { status: 200, body: url.includes('/-/fields') ? { fields: [] } : {} }
  );
  assert.deepEqual(
    await run(
      JSON.stringify(request('index-budget', { project: PROJECT, fields: [], composites: [] })),
      root,
      indexes
    ),
    { version: 1, error: 'provider-rejected', provider: { operation: 'indexes', status: 404 } }
  );
});

test('provider operation route table keeps resource names out of fixed diagnostic labels', () => {
  // Resource names deliberately resemble operation segments; query values are not route names.
  const project = 'projects/versions';
  const database = `${project}/databases/(default)`;
  const site = 'sites/channels';
  const version = `${site}/versions/files`;
  const cases = [
    ['firebase', project, 'project'],
    ['firebase', `${project}/webApps?pageSize=200`, 'web-apps'],
    ['firebase', `${project}/webApps/config/config`, 'web-app-config'],
    ['firebase', 'operations/webApps', 'web-app-operation'],
    ['firestore', database, 'database'],
    ['firestore', `${database}/collectionGroups/-/fields?filter=ttlConfig:*`, 'fields'],
    ['firestore', `${database}/collectionGroups/indexes/fields/sequence`, 'fields'],
    ['firestore', `${database}/collectionGroups/-/indexes?pageSize=200`, 'indexes'],
    ['firestore', `${database}/collectionGroups/fields/indexes`, 'indexes'],
    ['rules', `${project}/releases/cloud.firestore`, 'rules-release'],
    ['rules', `${project}/rulesets/releases`, 'rules-set'],
    ['auth', `${project}/config`, 'sign-in-config'],
    ['auth', `${project}/defaultSupportedIdpConfigs/google.com`, 'google-sign-in-config'],
    ['hosting', `${project}/sites/channels`, 'hosting-site'],
    ['hosting', `${site}/channels/live`, 'hosting-channel'],
    ['hosting', `${site}/versions?pageSize=100&pageToken=files`, 'hosting-version'],
    ['hosting', version, 'hosting-version'],
    ['hosting', `${version}/files?pageSize=200`, 'hosting-files'],
    ['hosting', `${site}/releases?versionName=sites/other/versions/files`, 'hosting-release'],
  ];
  for (const [service, route, expected] of cases) {
    assert.equal(providerOperation(service, route), expected, `${service}: ${route}`);
  }
});

test('ambiguous Hosting replies reconcile exact evidence once without repeating a mutation', async () => {
  for (const mode of ['missing-time', 'lost-create', 'populate-reply']) {
    const x = hostedFixture();
    const http = x.f.http;
    let lost = false;
    x.f.http = async (url, options, deadline) => {
      const response = await http(url, options, deadline);
      if (!lost && options.method === 'POST' && url.endsWith('/versions')) {
        if (mode === 'missing-time') {
          lost = true;
          const body = { ...response.body };
          delete body.createTime;
          return { ...response, body };
        }
        if (mode === 'lost-create') {
          lost = true;
          throw Error(CANARY);
        }
      }
      return response;
    };
    assert.equal((await x.call('apply-hosting', 'create')).state, 'done');
    assert.equal(
      x.calls.filter((c) => c.method === 'POST' && c.url.endsWith('/versions')).length,
      1
    );
    assert.ok(x.input.checkpoint.version);
    if (mode === 'populate-reply') {
      x.state.uploadUrl = 'https://invalid.example/not-an-upload';
      assert.equal((await x.call('apply-hosting', 'populate')).state, 'done');
      assert.equal(
        x.calls.filter((c) => c.method === 'POST' && c.url.endsWith(':populateFiles')).length,
        1
      );
      assert.ok(x.calls.every((c) => !c.url.includes('invalid.example')));
    }
  }
});

test('partial Hosting inventory records a closed list cause before any populate mutation', async () => {
  const x = hostedFixture();
  await x.call('apply-hosting', 'create');
  x.state.files = [{ path: '/different.html', hash: 'c'.repeat(64), status: 'EXPECTED' }];
  await assert.rejects(
    () => x.call('apply-hosting', 'populate'),
    (error) => {
      assert.deepEqual(error.cause, {
        code: 'files-mismatch',
        phase: 'list',
        httpStatusClass: '2xx',
        files: { planned: 1, expected: 1, active: 0, listed: 1, missing: 1, extra: 1 },
      });
      assert.ok(!JSON.stringify(error.cause).includes(PROJECT));
      return error.code === 'unknown';
    }
  );
  assert.equal(x.calls.filter((c) => c.url.endsWith(':populateFiles')).length, 0);
});

test('Hosting preparation time does not consume the mutation reply or reconciliation budgets', async () => {
  const x = hostedFixture();
  const http = x.f.http;
  let clock = Date.now();
  x.f.now = () => clock;
  x.f.deadline = clock + 90000;
  const start = clock;
  let postDeadline;
  x.f.http = async (url, options, deadline) => {
    if (url.includes('/versions') && options.method === 'GET') clock = start + 29000;
    if (url.endsWith('/versions') && options.method === 'POST') {
      postDeadline = deadline;
      clock = start + 30000;
      assert.ok(deadline > clock, 'a successful mutation reply has its own budget');
    }
    return http(url, options, deadline);
  };
  assert.equal((await x.call('apply-hosting', 'create')).state, 'done');
  assert.equal(postDeadline, start + 59000);
});

test('malformed HTTP rejection keeps only its fixed body cause and HTTP status class', async () => {
  const fetch = globalThis.fetch;
  globalThis.fetch = async () => new Response('PRIVATE_BODY', { status: 400 });
  try {
    await assert.rejects(
      () => fetchJson('https://offline.invalid/not-requested', {}, Date.now() + 1000),
      (error) => {
        assert.deepEqual(error.cause, {
          code: 'body-invalid',
          phase: 'body',
          httpStatusClass: '4xx',
        });
        assert.ok(!JSON.stringify(error).includes('PRIVATE_BODY'));
        return error.code === 'unknown';
      }
    );
  } finally {
    globalThis.fetch = fetch;
  }
});

test('Hosting HTTP refusals and unmatched ambiguity retain the cause without a second POST', async () => {
  for (const [status, code] of [
    [400, 'provider-rejected'],
    [403, 'permission-denied'],
    [429, 'quota-exceeded'],
    [408, 'unknown'],
    [409, 'unknown'],
    [500, 'unknown'],
  ]) {
    const x = hostedFixture();
    const http = x.f.http;
    let attempts = 0;
    x.f.http = async (url, options, deadline) => {
      if (options.method === 'POST' && url.endsWith('/versions')) {
        attempts++;
        return { status, body: {} };
      }
      return http(url, options, deadline);
    };
    await assert.rejects(
      () => x.call('apply-hosting', 'create'),
      (error) => {
        assert.equal(error.code, code);
        assert.deepEqual(error.cause, {
          code: 'http-rejected',
          phase: 'mutation',
          httpStatusClass: status < 500 ? '4xx' : '5xx',
        });
        return true;
      }
    );
    assert.equal(attempts, 1);
    assert.equal(x.input.checkpoint.version, null);
    assert.equal(
      x.calls.filter((c) => c.method === 'GET' && c.url.includes('/versions')).length,
      code === 'unknown' ? 2 : 1
    );
  }
});

test('a late successful mutation body gets one fresh-budget exact confirmation', async () => {
  const x = hostedFixture();
  const { DeployFailure } = require('../../src/deploy_firestore/helper.cjs');
  const http = x.f.http;
  const start = Date.now();
  let clock = start;
  let attempted = false;
  let confirmations = 0;
  x.f.now = () => clock;
  x.f.deadline = start + 90000;
  x.f.http = async (url, options, deadline) => {
    if (url.includes('/versions') && options.method === 'GET') {
      if (attempted) {
        confirmations++;
        assert.equal(deadline, start + 90000);
      } else clock = start + 29000;
    }
    const response = await http(url, options, deadline);
    if (url.endsWith('/versions') && options.method === 'POST') {
      assert.equal(deadline, start + 59000);
      attempted = true;
      clock = start + 60000;
      throw new DeployFailure('unknown', undefined, {
        code: 'deadline',
        phase: 'body',
        httpStatusClass: '2xx',
      });
    }
    return response;
  };
  assert.equal((await x.call('apply-hosting', 'create')).state, 'done');
  assert.equal(confirmations, 2); // One bounded run: list by identity, then exact version GET.
  assert.equal(x.calls.filter((c) => c.method === 'POST' && c.url.endsWith('/versions')).length, 1);
});

test('Hosting populate observes the union of partially ACTIVE and remaining files without another POST', async () => {
  for (const allActive of [false, true]) {
    const x = hostedFixture();
    x.input.hosting.content.files.push({
      ...x.input.hosting.content.files[0],
      path: '/other.html',
      gzipDigest: hash('other'),
    });
    await x.call('apply-hosting', 'create');
    x.state.files = x.input.hosting.content.files.map((file, i) => ({
      path: file.path,
      hash: file.gzipDigest,
      status: allActive || i === 0 ? 'ACTIVE' : 'EXPECTED',
    }));
    assert.equal((await x.call('observe-hosting', 'populate')).state, 'satisfied');
    assert.equal((await x.call('apply-hosting', 'populate')).state, 'done');
    assert.equal(x.calls.filter((c) => c.url.endsWith(':populateFiles')).length, 0);
    const statuses = x.calls
      .filter((c) => new URL(c.url).pathname.endsWith('/files'))
      .map((c) => new URL(c.url).searchParams.get('status'));
    assert.deepEqual(statuses, ['EXPECTED', 'ACTIVE', 'EXPECTED', 'ACTIVE']);
  }
});

test('Hosting accepts omitted or empty uploadRequiredHashes without a reconciliation read or upload', async () => {
  for (const omitRequired of [false, true]) {
    const x = hostedFixture();
    await x.call('apply-hosting', 'create');
    x.state.activeHashes = x.input.hosting.content.files.map((file) => file.gzipDigest);
    x.state.omitRequired = omitRequired;
    const http = x.f.http;
    let populated = false;
    x.f.http = async (url, options, deadline) => {
      assert.ok(!populated, 'a valid populate answer needs no reconciliation');
      const result = await http(url, options, deadline);
      if (url.endsWith(':populateFiles')) populated = true;
      return result;
    };
    assert.equal((await x.call('apply-hosting', 'populate')).state, 'done');
    assert.equal(x.calls.filter((c) => c.url.endsWith(':populateFiles')).length, 1);
    assert.equal(
      x.calls.filter((c) => c.url.startsWith('https://upload-firebasehosting.')).length,
      0
    );
  }
});

test('Hosting all-ACTIVE files skip upload and complete finalize, release and verification', async () => {
  const x = hostedFixture();
  for (const action of ['web-app', 'site', 'configure', 'create'])
    await x.call('apply-hosting', action);
  x.state.activeHashes = x.input.hosting.content.files.map((file) => file.gzipDigest);
  await x.call('apply-hosting', 'populate');
  assert.equal(
    (
      await x.call('apply-hosting', 'upload', {
        hash: hash('gzip-fixture'),
        bytesBase64: Buffer.from('gzip-fixture').toString('base64'),
      })
    ).state,
    'done'
  );
  assert.equal(
    x.calls.filter((c) => c.url.startsWith('https://upload-firebasehosting.')).length,
    0
  );
  assert.equal((await x.call('apply-hosting', 'finalize')).state, 'done');
  assert.equal((await x.call('apply-hosting', 'release')).state, 'done');
  assert.equal((await x.call('verify-hosting', 'verify')).siteAppId, x.state.site.appId);
  const reads = x.calls.filter((c) => new URL(c.url).pathname.endsWith('/files'));
  assert.ok(reads.length > 0);
  assert.ok(
    reads.every((c) => ['EXPECTED', 'ACTIVE'].includes(new URL(c.url).searchParams.get('status')))
  );
});

test('Hosting file union handles a status transition and preserves each status on bounded pagination', async () => {
  for (const mode of ['transition', 'pages']) {
    const x = hostedFixture();
    for (const path of ['/other.html', '/third.html', '/fourth.html'])
      x.input.hosting.content.files.push({
        ...x.input.hosting.content.files[0],
        path,
        gzipDigest: hash(path),
      });
    x.input.hosting.content.files.sort((a, b) => a.path.localeCompare(b.path));
    await x.call('apply-hosting', 'create');
    x.state.files = x.input.hosting.content.files.map((file, i) => ({
      path: file.path,
      hash: file.gzipDigest,
      status: mode === 'pages' && i > 1 ? 'ACTIVE' : 'EXPECTED',
    }));
    const http = x.f.http;
    x.f.http = async (url, options, deadline) => {
      const response = await http(url, options, deadline);
      if (new URL(url).pathname.endsWith('/files')) {
        const query = new URL(url).searchParams;
        assert.ok(['EXPECTED', 'ACTIVE'].includes(query.get('status')));
        if (mode === 'transition' && query.get('status') === 'EXPECTED')
          x.state.files[0] = { ...x.state.files[0], status: 'ACTIVE' };
        if (mode === 'pages') {
          const page = query.has('pageToken')
            ? { files: response.body.files.slice(1) }
            : { files: response.body.files.slice(0, 1), nextPageToken: 'second' };
          return { status: 200, body: page };
        }
      }
      return response;
    };
    assert.equal((await x.call('apply-hosting', 'populate')).state, 'done');
    assert.equal(x.calls.filter((c) => c.url.endsWith(':populateFiles')).length, 0);
    if (mode === 'pages') {
      const continued = x.calls.filter(
        (c) => new URL(c.url).searchParams.get('pageToken') === 'second'
      );
      assert.deepEqual(
        continued.map((c) => new URL(c.url).searchParams.get('status')),
        ['EXPECTED', 'ACTIVE']
      );
    }
  }
});

test('Hosting union refuses conflicting hashes, duplicate paths, wrong statuses and repeated tokens without a mutation', async () => {
  for (const mode of ['conflict', 'duplicate', 'status', 'token']) {
    const x = hostedFixture();
    await x.call('apply-hosting', 'create');
    x.state.files = [{ path: '/index.html', hash: hash('gzip-fixture'), status: 'EXPECTED' }];
    const http = x.f.http;
    x.f.http = async (url, options, deadline) => {
      const response = await http(url, options, deadline);
      const query = new URL(url).searchParams;
      if (!new URL(url).pathname.endsWith('/files')) return response;
      if (mode === 'conflict' && query.get('status') === 'ACTIVE')
        return {
          status: 200,
          body: { files: [{ ...x.state.files[0], hash: 'd'.repeat(64), status: 'ACTIVE' }] },
        };
      if (mode === 'duplicate' && response.body.files.length)
        response.body.files.push(response.body.files[0]);
      if (mode === 'status' && response.body.files.length)
        response.body.files[0] = { ...response.body.files[0], status: 'BROKEN' };
      if (mode === 'token') response.body.nextPageToken = 'repeated';
      return response;
    };
    await failure(() => x.call('apply-hosting', 'populate'), 'unknown');
    assert.equal(x.calls.filter((c) => c.url.endsWith(':populateFiles')).length, 0);
    assert.ok(x.calls.filter((c) => new URL(c.url).pathname.endsWith('/files')).length <= 2);
  }
});

test('Hosting finalize waits only for known remaining uploads, and refuses an incomplete union', async () => {
  const x = hostedFixture();
  await x.call('apply-hosting', 'create');
  await x.call('apply-hosting', 'populate');
  assert.equal((await x.call('apply-hosting', 'finalize')).state, 'building');
  x.state.files = [];
  await assert.rejects(
    () => x.call('apply-hosting', 'finalize'),
    (error) => {
      assert.deepEqual(error.cause.files, {
        planned: 1,
        expected: 0,
        active: 0,
        listed: 0,
        missing: 1,
        extra: 0,
      });
      return error.code === 'unknown';
    }
  );
  assert.equal(x.calls.filter((c) => c.method === 'PATCH').length, 0);
});

test('Hosting malformed populate hash requirements stay unknown without following an upload URL', async () => {
  for (const invalid of [null, {}, ['d'.repeat(64)]]) {
    const x = hostedFixture();
    await x.call('apply-hosting', 'create');
    const http = x.f.http;
    let populated = false;
    x.f.http = async (url, options, deadline) => {
      if (populated) throw Error(CANARY);
      const response = await http(url, options, deadline);
      if (url.endsWith(':populateFiles')) {
        populated = true;
        response.body.uploadRequiredHashes = invalid;
      }
      return response;
    };
    await failure(() => x.call('apply-hosting', 'populate'), 'unknown');
    assert.equal(x.calls.filter((c) => c.url.endsWith(':populateFiles')).length, 1);
    assert.equal(
      x.calls.filter((c) => c.url.startsWith('https://upload-firebasehosting.')).length,
      0
    );
  }
});
