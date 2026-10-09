'use strict';
// Embedded in tmt-remote, evaluated by Node, never opened from a runtime file path.
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const SUPPORTED = ['15.29.0'];
const BYTES = 4 * 1024 * 1024;
const PAGES = 10;
const FAULTS = new Set([
  'unsupported-tool',
  'login-required',
  'account-changed',
  'permission-denied',
  'api-disabled',
  'quota-exceeded',
  'database-mismatch',
  'rules-foreign',
  'provider-rejected',
  'unknown',
]);
class DeployFailure extends Error {
  constructor(code) {
    super('Firebase request could not be confirmed');
    this.code = code;
  }
}
const fail = (code) => {
  throw new DeployFailure(code);
};
const sha = (source) => crypto.createHash('sha256').update(source).digest('hex');
function keys(value, expected) {
  if (
    !value ||
    typeof value !== 'object' ||
    Array.isArray(value) ||
    Object.keys(value).sort().join('|') !== [...expected].sort().join('|')
  )
    fail('provider-rejected');
}
function validate(request) {
  keys(request, ['version', 'operation', 'input', 'account', 'budgetMs']);
  if (
    request.version !== 1 ||
    !Number.isInteger(request.budgetMs) ||
    request.budgetMs < 1 ||
    request.budgetMs > 30000
  )
    fail('provider-rejected');
  if (
    request.account !== null &&
    (typeof request.account !== 'string' ||
      !request.account ||
      request.account.length > 1024 ||
      /[\x00-\x1f\x7f]/.test(request.account))
  )
    fail('provider-rejected');
  const fields = {
    compatibility: [],
    account: [],
    'live-rules': ['project'],
    'observe-database': ['project', 'location'],
    'apply-database': ['project', 'location'],
    'observe-sign-in': ['project', 'provider'],
    'apply-sign-in': ['project', 'provider'],
    'observe-index': ['project', 'collection', 'field', 'direction'],
    'apply-index': ['project', 'collection', 'field', 'direction'],
    'observe-rules': ['project', 'source', 'deployment', 'replacedDigest'],
    'apply-rules': ['project', 'source', 'deployment', 'replacedDigest'],
  };
  if (!Object.hasOwn(fields, request.operation)) fail('provider-rejected');
  keys(request.input, fields[request.operation]);
  const i = request.input;
  if (i.project !== undefined && !/^[a-z][a-z0-9-]{4,28}[a-z0-9]$/.test(i.project))
    fail('provider-rejected');
  if (i.location !== undefined && !/^[a-z0-9-]{1,32}$/.test(i.location)) fail('provider-rejected');
  if (i.provider !== undefined && !['anonymous', 'google.com'].includes(i.provider))
    fail('provider-rejected');
  if (i.collection !== undefined && !/^[a-z0-9_-]{1,64}$/.test(i.collection))
    fail('provider-rejected');
  if (i.field !== undefined && !/^[A-Za-z][A-Za-z0-9]{0,63}$/.test(i.field))
    fail('provider-rejected');
  if (i.direction !== undefined && !['asc', 'desc'].includes(i.direction))
    fail('provider-rejected');
  if (i.source !== undefined) {
    if (
      typeof i.source !== 'string' ||
      Buffer.byteLength(i.source) > BYTES ||
      !/^[0-9a-f-]{36}$/.test(i.deployment)
    )
      fail('provider-rejected');
    const line = `// tmt-remote deployment ${i.deployment} rules `;
    const split = i.source.indexOf('\n');
    if (
      !i.source.startsWith(line) ||
      i.source.slice(line.length, split) !== sha(i.source.slice(split + 1))
    )
      fail('provider-rejected');
    if (i.replacedDigest !== null && !/^[0-9a-f]{64}$/.test(i.replacedDigest))
      fail('provider-rejected');
  }
  if (request.operation.startsWith('apply-') && request.account === null) fail('provider-rejected');
  return request;
}
function compatibility(root) {
  if (!path.isAbsolute(root)) fail('unsupported-tool');
  let pkg;
  try {
    const bytes = fs.readFileSync(path.join(root, 'package.json'));
    if (bytes.length > 65536) fail('unsupported-tool');
    pkg = JSON.parse(bytes);
  } catch {
    fail('unsupported-tool');
  }
  if (pkg.name !== 'firebase-tools' || !SUPPORTED.includes(pkg.version)) fail('unsupported-tool');
  for (const file of [
    'lib/auth.js',
    'lib/logger.js',
    'lib/api.js',
    'lib/apiv2.js',
    'lib/configstore.js',
  ]) {
    try {
      const p = fs.realpathSync(path.join(root, file));
      if (!p.startsWith(fs.realpathSync(root) + path.sep) || !fs.statSync(p).isFile())
        fail('unsupported-tool');
    } catch {
      fail('unsupported-tool');
    }
  }
}
function credentials(root) {
  // No CLI initialization, debug-file logger, ADC or environment token fallback.
  const logger = require(path.join(root, 'lib/logger.js')).logger;
  if (!logger || typeof logger.clear !== 'function') fail('unsupported-tool');
  logger.clear();
  logger.silent = true;
  const auth = require(path.join(root, 'lib/auth.js'));
  if (typeof auth.selectAccount !== 'function' || typeof auth.getAccessToken !== 'function')
    fail('unsupported-tool');
  return async () => {
    const selected = auth.selectAccount();
    if (!selected?.user?.email || !selected?.tokens?.refresh_token) fail('login-required');
    const tokens = await auth.getAccessToken(selected.tokens.refresh_token, [
      'https://www.googleapis.com/auth/cloud-platform',
      'https://www.googleapis.com/auth/firebase',
      'openid',
      'email',
    ]);
    if (typeof tokens?.access_token !== 'string' || !tokens.access_token) fail('login-required');
    return { account: selected.user.email, token: tokens.access_token };
  };
}
function fault(response, body) {
  if (body?.error?.details?.some((d) => d.reason === 'SERVICE_DISABLED')) return 'api-disabled';
  if (response.status === 403) return 'permission-denied';
  if (response.status === 429) return 'quota-exceeded';
  if (response.status >= 500 || response.status === 408 || response.status === 409)
    return 'unknown';
  return 'provider-rejected';
}
async function fetchJson(url, options, deadline) {
  const left = deadline - Date.now();
  if (left <= 0) fail('unknown');
  const response = await fetch(url, {
    ...options,
    redirect: 'error',
    signal: AbortSignal.timeout(Math.min(left, 20000)),
  });
  let size = 0;
  const chunks = [];
  for await (const part of response.body ?? []) {
    size += part.length;
    if (size > BYTES) {
      await response.body?.cancel().catch(() => {});
      fail('unknown');
    }
    chunks.push(part);
  }
  let value;
  try {
    value = JSON.parse(Buffer.concat(chunks).toString('utf8'));
  } catch {
    fail('unknown');
  }
  if (Date.now() >= deadline) fail('unknown');
  return { status: response.status, body: value };
}
async function execute(request, deps) {
  const r = validate(request),
    i = r.input;
  if (r.operation === 'compatibility')
    return { versions: SUPPORTED, maxBytes: BYTES, maxPages: PAGES };
  const deadline = deps.deadline;
  let effect = false;
  async function identity() {
    const credential = await deps.credential();
    // Independently verify the token's live identity, not just configstore's label.
    const actual = await deps.http(
      'https://www.googleapis.com/oauth2/v3/userinfo',
      { method: 'GET', headers: { Authorization: `Bearer ${credential.token}` } },
      deadline
    );
    if (
      actual.status !== 200 ||
      actual.body?.email !== credential.account ||
      actual.body?.email_verified !== true
    )
      fail('login-required');
    if (r.account !== null && credential.account !== r.account) fail('account-changed');
    return credential;
  }
  let credential = await identity();
  if (r.operation === 'account') return credential.account;
  const project = i.project;
  const bases = {
    firebase: 'https://firebase.googleapis.com/v1beta1',
    firestore: 'https://firestore.googleapis.com/v1',
    auth: 'https://identitytoolkit.googleapis.com/admin/v2',
    rules: 'https://firebaserules.googleapis.com/v1',
  };
  async function api(service, route, method = 'GET', body, absent = false) {
    const earlierEffect = effect;
    if (method !== 'GET') {
      credential = await identity();
      effect = true;
    }
    const reply = await deps.http(
      `${bases[service]}/${route}`,
      {
        method,
        headers: {
          Authorization: `Bearer ${credential.token}`,
          'Content-Type': 'application/json',
        },
        ...(body === undefined ? {} : { body: JSON.stringify(body) }),
      },
      deadline
    );
    if (absent && reply.status === 404) return null;
    if (reply.status < 200 || reply.status >= 300) {
      const code = fault(reply, reply.body);
      if (method !== 'GET' && code !== 'unknown' && !earlierEffect) effect = false;
      fail(code);
    }
    return reply.body;
  }
  async function pages(service, route, key) {
    const all = [];
    let token = '';
    const seen = new Set();
    for (let page = 0; page < PAGES; page++) {
      const result = await api(
        service,
        `${route}${route.includes('?') ? '&' : '?'}pageSize=200${token ? `&pageToken=${encodeURIComponent(token)}` : ''}`
      );
      if (!result || (result[key] !== undefined && !Array.isArray(result[key]))) fail('unknown');
      if ((result[key] ?? []).length > 200) fail('unknown');
      all.push(...(result[key] ?? []));
      if (Buffer.byteLength(JSON.stringify(all)) > BYTES) fail('unknown');
      token = result.nextPageToken ?? '';
      if (typeof token !== 'string' || token.length > 4096) fail('unknown');
      if (!token) return all;
      if (seen.has(token)) fail('unknown');
      seen.add(token);
    }
    fail('unknown');
  }
  const database = `projects/${project}/databases/(default)`;
  const release = `projects/${project}/releases/cloud.firestore`;
  async function liveRules() {
    const current = await api('rules', release, 'GET', undefined, true);
    if (current === null) return null;
    if (
      typeof current.rulesetName !== 'string' ||
      !current.rulesetName.startsWith(`projects/${project}/rulesets/`)
    )
      fail('unknown');
    const ruleset = await api('rules', current.rulesetName);
    const files = ruleset?.source?.files;
    if (
      !Array.isArray(files) ||
      files.length !== 1 ||
      files[0].name !== 'firestore.rules' ||
      typeof files[0].content !== 'string'
    )
      fail('rules-foreign');
    return files[0].content;
  }
  function safeRules(source) {
    if (source === null || source === i.source) return true;
    const prefix = `// tmt-remote deployment ${i.deployment} rules `,
      end = source.indexOf('\n');
    if (
      source.startsWith(prefix) &&
      source.slice(prefix.length, end) === sha(source.slice(end + 1))
    )
      return true;
    return sha(source) === i.replacedDigest;
  }
  try {
    // Validate an existing Firebase project; this adapter never creates or enables one.
    const p = await api('firebase', `projects/${project}`);
    if (p.projectId !== project || p.state !== 'ACTIVE') fail('provider-rejected');
    if (r.operation === 'live-rules' || r.operation === 'observe-rules') return await liveRules();
    if (r.operation.endsWith('database')) {
      const existing = await api('firestore', database, 'GET', undefined, true);
      if (existing !== null) {
        const mismatch =
          existing.name !== database ||
          existing.locationId !== i.location ||
          existing.type !== 'FIRESTORE_NATIVE' ||
          (existing.databaseEdition ?? 'STANDARD') !== 'STANDARD';
        if (mismatch) {
          if (r.operation.startsWith('apply')) fail('database-mismatch');
          return 'mismatch';
        }
        return r.operation.startsWith('apply')
          ? 'done'
          : existing.createTime
            ? 'adopted'
            : 'building';
      }
      if (r.operation.startsWith('observe')) return 'absent';
      await api('firestore', `projects/${project}/databases?databaseId=%28default%29`, 'POST', {
        locationId: i.location,
        type: 'FIRESTORE_NATIVE',
        databaseEdition: 'STANDARD',
      });
      return 'done'; // The engine's final read-back must confirm operation completion.
    }
    if (r.operation.endsWith('sign-in')) {
      const config = await api('auth', `projects/${project}/config`, 'GET', undefined, true);
      if (config === null) return 'initialize-auth';
      if (i.provider === 'google.com') {
        const google = await api(
          'auth',
          `projects/${project}/defaultSupportedIdpConfigs/google.com`,
          'GET',
          undefined,
          true
        );
        return google?.enabled === true
          ? r.operation.startsWith('apply')
            ? 'done'
            : 'adopted'
          : 'enable-google-sign-in';
      }
      if (config.signIn?.anonymous?.enabled === true)
        return r.operation.startsWith('apply') ? 'done' : 'adopted';
      if (r.operation.startsWith('observe')) return 'absent';
      await api('auth', `projects/${project}/config?updateMask=signIn.anonymous.enabled`, 'PATCH', {
        signIn: { anonymous: { enabled: true } },
      });
      return 'done';
    }
    if (r.operation.endsWith('index')) {
      const name = `${database}/collectionGroups/${i.collection}/fields/${i.field}`;
      const existing = await api('firestore', name);
      if (
        existing.name !== name ||
        !existing.indexConfig ||
        !Array.isArray(existing.indexConfig.indexes)
      )
        fail('unknown');
      const order = i.direction === 'asc' ? 'ASCENDING' : 'DESCENDING';
      const matched = existing.indexConfig.indexes.find(
        (x) =>
          x.queryScope === 'COLLECTION' &&
          x.fields?.length === 1 &&
          x.fields[0].fieldPath === i.field &&
          x.fields[0].order === order
      );
      if (matched) {
        if (matched.state === 'CREATING')
          return r.operation.startsWith('apply') ? 'done' : 'building';
        if (matched.state !== 'READY') fail('provider-rejected');
        return r.operation.startsWith('apply') ? 'done' : 'adopted';
      }
      if (r.operation.startsWith('observe')) return 'absent';
      const configured = await pages(
        'firestore',
        `${database}/collectionGroups/-/fields?filter=${encodeURIComponent('indexConfig.usesAncestorConfig=false OR ttlConfig:*')}`,
        'fields'
      );
      const unique = new Set(configured.map((x) => x.name));
      if (configured.some((x) => typeof x.name !== 'string') || unique.size !== configured.length)
        fail('unknown');
      if (!unique.has(name) && unique.size >= 200) fail('quota-exceeded');
      // Preserve every existing index and TTL policy. updateMask excludes TTL.
      const indexes = existing.indexConfig.indexes.map((x) =>
        Object.fromEntries(
          Object.entries(x).filter(([key]) =>
            ['queryScope', 'fields', 'apiScope', 'density', 'multikey', 'unique'].includes(key)
          )
        )
      );
      indexes.push({ queryScope: 'COLLECTION', fields: [{ fieldPath: i.field, order }] });
      await api('firestore', `${name}?updateMask=indexConfig`, 'PATCH', {
        name,
        indexConfig: { indexes },
      });
      return 'done';
    }
    if (r.operation === 'apply-rules') {
      const before = await liveRules();
      if (!safeRules(before)) fail('rules-foreign');
      if (before === i.source) return 'done';
      // An earlier unknown create is recovered by exact digest/source before a new POST.
      const listed = await pages('rules', `projects/${project}/rulesets`, 'rulesets');
      let found = null;
      for (const item of listed) {
        if (typeof item.name !== 'string' || !item.name.startsWith(`projects/${project}/rulesets/`))
          fail('unknown');
        const candidate = await api('rules', item.name);
        if (
          !Array.isArray(candidate?.source?.files) ||
          candidate.source.files.some(
            (f) => typeof f.name !== 'string' || typeof f.content !== 'string'
          )
        )
          fail('unknown');
        if (
          candidate.source.files.length === 1 &&
          candidate.source.files[0].name === 'firestore.rules' &&
          candidate.source.files[0].content === i.source
        ) {
          found = item.name;
          break;
        }
      }
      if (found === null) {
        const created = await api('rules', `projects/${project}/rulesets`, 'POST', {
          source: { files: [{ name: 'firestore.rules', content: i.source }] },
        });
        if (
          typeof created.name !== 'string' ||
          !created.name.startsWith(`projects/${project}/rulesets/`)
        )
          fail('unknown');
        found = created.name;
      }
      // No provider CAS exists: single-writer project is required. Detect visible drift.
      const latest = await liveRules();
      if (latest !== before) {
        if (effect) fail('unknown');
        fail('rules-foreign');
      }
      await api(
        'rules',
        latest === null ? `projects/${project}/releases` : release,
        latest === null ? 'POST' : 'PATCH',
        latest === null
          ? { name: release, rulesetName: found }
          : { release: { name: release, rulesetName: found }, updateMask: 'rulesetName' }
      );
      if ((await liveRules()) !== i.source) fail('unknown');
      return 'done';
    }
    fail('provider-rejected');
  } catch (error) {
    if (effect) fail('unknown');
    if (error instanceof DeployFailure) throw error;
    fail('provider-rejected');
  }
}
async function run(raw, root, deps) {
  try {
    if (Buffer.byteLength(raw) > BYTES) fail('provider-rejected');
    const request = validate(JSON.parse(raw));
    compatibility(root);
    if (request.operation === 'compatibility')
      return { version: 1, result: { versions: SUPPORTED, maxBytes: BYTES, maxPages: PAGES } };
    const deadline = Date.now() + request.budgetMs;
    const result = await execute(
      request,
      deps ?? { credential: credentials(root), http: fetchJson, deadline }
    );
    const output = { version: 1, result };
    if (Buffer.byteLength(JSON.stringify(output)) > BYTES) fail('unknown');
    return output;
  } catch (error) {
    return {
      version: 1,
      error: error instanceof DeployFailure && FAULTS.has(error.code) ? error.code : 'unknown',
    };
  }
}
async function main() {
  const emit = process.stdout.write.bind(process.stdout);
  // Suppress all dependency output before loading it; exceptions never cross the boundary.
  process.stdout.write = () => true;
  process.stderr.write = () => true;
  let raw = '',
    size = 0;
  for await (const chunk of process.stdin) {
    size += chunk.length;
    if (size > BYTES) {
      emit(JSON.stringify({ version: 1, error: 'provider-rejected' }));
      return;
    }
    raw += chunk.toString('utf8');
  }
  emit(JSON.stringify(await run(raw, process.argv[1])));
}
module.exports = {
  SUPPORTED,
  BYTES,
  PAGES,
  DeployFailure,
  validate,
  compatibility,
  execute,
  run,
  fetchJson,
};
if (require.main === module || require.main === undefined)
  main().catch(() => {
    /* Parent sees failed/empty output as unknown. */ process.exitCode = 1;
  });
