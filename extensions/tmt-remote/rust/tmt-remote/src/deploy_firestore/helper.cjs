'use strict';
// Embedded in tmt-remote, evaluated by Node, never opened from a runtime file path.
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const SUPPORTED = ['15.29.0'];
const BYTES = 4 * 1024 * 1024;
const PAGES = 10;
const UPLOAD_BYTES = Math.ceil((4 * 1024 * 1024 + 65536) / 3) * 4 + 256 * 1024;
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
    'hosting-inventory': ['project'],
    ...Object.fromEntries(
      ['observe-hosting', 'apply-hosting', 'verify-hosting'].map((op) => [
        op,
        [
          'project',
          'deployment',
          'planDigest',
          'hosting',
          'checkpoint',
          'steps',
          'action',
          'hash',
          'bytesBase64',
          'source',
        ],
      ])
    ),
    'live-rules': ['project'],
    'index-budget': ['project', 'fields'],
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
  if (
    i.fields !== undefined &&
    (!Array.isArray(i.fields) ||
      i.fields.length > 200 ||
      i.fields.some(
        (field) =>
          typeof field !== 'string' || !/^[a-z0-9_-]{1,64}\/[A-Za-z][A-Za-z0-9]{0,63}$/.test(field)
      ) ||
      new Set(i.fields).size !== i.fields.length)
  )
    fail('provider-rejected');
  if (request.operation.endsWith('-hosting')) {
    if (
      !/^[0-9a-f]{64}$/.test(i.planDigest) ||
      !/^[0-9a-f-]{36}$/.test(i.deployment) ||
      !i.hosting ||
      !i.checkpoint ||
      !Array.isArray(i.steps) ||
      ![
        'web-app',
        'site',
        'create',
        'populate',
        'upload',
        'finalize',
        'release',
        'verify',
      ].includes(i.action) ||
      (i.hash !== null && !/^[0-9a-f]{64}$/.test(i.hash)) ||
      (i.bytesBase64 !== null &&
        (typeof i.bytesBase64 !== 'string' || i.bytesBase64.length > UPLOAD_BYTES))
    )
      fail('provider-rejected');
    if (
      !Array.isArray(i.hosting.content?.files) ||
      i.hosting.content.files.length > 256 ||
      Buffer.byteLength(JSON.stringify(i.hosting.content.config)) > 65536
    )
      fail('provider-rejected');
  }
  if (i.source !== undefined && !request.operation.endsWith('-hosting')) {
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
    const bytes = Buffer.concat(chunks);
    value =
      bytes.length === 0 && response.status >= 200 && response.status < 300
        ? null
        : JSON.parse(bytes.toString('utf8'));
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
    hosting: 'https://firebasehosting.googleapis.com/v1beta1',
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
        `${route}${route.includes('?') ? '&' : '?'}pageSize=${service === 'hosting' && key === 'versions' ? 100 : 200}${token ? `&pageToken=${encodeURIComponent(token)}` : ''}`
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
  // Hosting uses the same credential/effect fence and bounded requests as Firestore.
  const stable = (value) => JSON.stringify(normalize(value));
  function normalize(value) {
    if (Array.isArray(value)) return value.map(normalize);
    if (value && typeof value === 'object')
      return Object.fromEntries(
        Object.keys(value)
          .sort()
          .map((k) => [k, normalize(value[k])])
      );
    return value;
  }
  const appName = () => `tmt Remote (${i.deployment})`;
  function resource(value, prefix) {
    if (
      typeof value !== 'string' ||
      value.length > 256 ||
      !value.startsWith(prefix) ||
      !/^[A-Za-z0-9_:-]+$/.test(value.slice(prefix.length))
    )
      fail('unknown');
    return value;
  }
  async function apps() {
    const found = await pages('firebase', `projects/${project}/webApps`, 'apps');
    if (found.length > 256) fail('unknown');
    return found.map((a) => {
      if (
        a.projectId !== project ||
        typeof a.appId !== 'string' ||
        typeof a.displayName !== 'string' ||
        typeof a.state !== 'string'
      )
        fail('unknown');
      resource(a.name, `projects/${project}/webApps/`);
      return { id: a.appId, displayName: a.displayName, state: a.state };
    });
  }
  async function liveHosting() {
    const site = await api(
      'hosting',
      `projects/${project}/sites/${project}`,
      'GET',
      undefined,
      true
    );
    if (site === null) return { siteExists: false, webApps: await apps(), live: null };
    if (
      site.name !== `projects/${project}/sites/${project}` ||
      site.defaultUrl !== `https://${project}.web.app`
    )
      fail('unknown');
    const channel = await api('hosting', `sites/${project}/channels/live`, 'GET', undefined, true);
    let live = null;
    if (channel?.release) {
      const name = resource(channel.release.version?.name, `sites/${project}/versions/`);
      const version = await api('hosting', name);
      if (version.status !== 'FINALIZED') fail('unknown');
      const files = await pages('hosting', `${name}/files`, 'files');
      if (
        files.length > 256 ||
        files.some(
          (f) =>
            f.status !== 'ACTIVE' || !/^[0-9a-f]{64}$/.test(f.hash) || typeof f.path !== 'string'
        )
      )
        fail('unknown');
      files.sort((a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0));
      if (new Set(files.map((f) => f.path)).size !== files.length) fail('unknown');
      const config = version.config;
      if (!config || Buffer.byteLength(stable(config)) > 65536) fail('unknown');
      live = {
        version: name,
        deploymentId: version.labels?.['tmt-deployment'] ?? null,
        planDigestPrefix: version.labels?.['tmt-plan'] ?? null,
        contentDigest: version.labels?.['tmt-content'] ?? null,
        files: files.map(({ path, hash, status }) => ({ path, hash, status })),
        config,
      };
    }
    return { siteExists: true, webApps: await apps(), live };
  }
  if (r.operation === 'hosting-inventory') return liveHosting();
  async function hosting() {
    const cp = structuredClone(i.checkpoint);
    const h = i.hosting;
    const files = h.content.files.map((f) => ({
      path: f.path,
      hash: f.gzipDigest,
      status: 'ACTIVE',
    }));
    const contentDigest = sha(stable({ files, config: h.content.config }));
    const labels = {
      'tmt-deployment': i.deployment,
      'tmt-plan': i.planDigest.slice(0, 12),
      'tmt-content': contentDigest,
    };
    const reply = (state) => ({ state, checkpoint: cp });
    const applying = r.operation === 'apply-hosting';
    async function selectedApp() {
      const listed = await apps();
      if (cp.appId !== null) {
        const matches = listed.filter((a) => a.id === cp.appId && a.state === 'ACTIVE');
        if (matches.length !== 1) fail('unknown');
        return matches[0];
      }
      const matches = listed.filter((a) => a.state === 'ACTIVE' && a.displayName === appName());
      if (matches.length > 1) fail('unknown');
      if (matches.length === 1) {
        cp.appId = matches[0].id;
        cp.operation = null;
        return matches[0];
      }
      return null;
    }
    async function stageVersion() {
      if (cp.version !== null) {
        resource(cp.version, `sites/${project}/versions/`);
        const found = await api('hosting', cp.version, 'GET', undefined, true);
        if (found === null || found.status === 'ABANDONED' || found.status === 'DELETED')
          fail('unknown');
        if (
          found.status === 'CREATED' &&
          (!Number.isFinite(cp.versionCreatedMs) ||
            Date.now() - cp.versionCreatedMs >= 12 * 60 * 60 * 1000)
        )
          fail('unknown');
        if (
          stable(found.config) !== stable(h.content.config) ||
          stable(found.labels) !== stable(labels)
        )
          fail('unknown');
        return found;
      }
      const versions = await pages('hosting', `sites/${project}/versions`, 'versions');
      const matches = versions.filter(
        (v) => stable(v.labels) === stable(labels) && stable(v.config) === stable(h.content.config)
      );
      if (matches.length > 1) fail('unknown');
      if (!matches.length) return null;
      const found = matches[0];
      cp.version = resource(found.name, `sites/${project}/versions/`);
      cp.versionCreatedMs = Date.parse(found.createTime);
      return stageVersion();
    }
    async function versionFiles() {
      const listed = await pages(
        'hosting',
        `${resource(cp.version, `sites/${project}/versions/`)}/files`,
        'files'
      );
      if (listed.length > 256 || new Set(listed.map((f) => f.path)).size !== listed.length)
        fail('unknown');
      const sorted = listed
        .map(({ path, hash, status }) => ({ path, hash, status }))
        .sort((a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0));
      return sorted;
    }
    async function exactLive() {
      const live = (await liveHosting()).live;
      return (
        live &&
        live.version === cp.version &&
        stable(live.files) === stable(files) &&
        stable(live.config) === stable(h.content.config) &&
        live.deploymentId === i.deployment &&
        live.planDigestPrefix === i.planDigest.slice(0, 12) &&
        live.contentDigest === contentDigest
      );
    }
    if (i.action === 'web-app') {
      if (await selectedApp()) return reply(applying ? 'done' : 'satisfied');
      if (cp.operation !== null) {
        resource(cp.operation, 'operations/');
        const op = await api('firebase', cp.operation, 'GET', undefined, true);
        if (op?.error) fail('unknown');
        if (op === null) fail('unknown');
        if (!op.done) return reply('building');
        if (!(await selectedApp())) fail('unknown');
        return reply(applying ? 'done' : 'satisfied');
      }
      if (!applying) return reply('absent');
      if (!h.createWebApp) fail('provider-rejected');
      const operation = await api('firebase', `projects/${project}/webApps`, 'POST', {
        displayName: appName(),
      });
      cp.operation = resource(operation.name, 'operations/');
      return reply('building');
    }
    if (i.action === 'site') {
      const site = await api(
        'hosting',
        `projects/${project}/sites/${project}`,
        'GET',
        undefined,
        true
      );
      if (site !== null) {
        if (site.name !== `projects/${project}/sites/${project}` || site.defaultUrl !== h.publicUrl)
          fail('unknown');
        return reply(applying ? 'done' : 'satisfied');
      }
      if (!applying) return reply('absent');
      if (!h.createSite) fail('provider-rejected');
      await api('hosting', `projects/${project}/sites?siteId=${project}`, 'POST', {});
      return reply('done');
    }
    if (i.action === 'create') {
      if (await stageVersion()) return reply(applying ? 'done' : 'satisfied');
      if (!applying) return reply('absent');
      const version = await api('hosting', `sites/${project}/versions`, 'POST', {
        config: h.content.config,
        labels,
      });
      cp.version = resource(version.name, `sites/${project}/versions/`);
      cp.versionCreatedMs = Date.parse(version.createTime);
      if (!Number.isFinite(cp.versionCreatedMs)) fail('unknown');
      return reply('done');
    }
    const version = await stageVersion();
    if (version === null) fail('unknown');
    if (i.action === 'populate') {
      const listed = await versionFiles();
      const expected = files.map(({ path, hash }) => ({ path, hash }));
      if (stable(listed.map(({ path, hash }) => ({ path, hash }))) === stable(expected))
        return reply(applying ? 'done' : 'satisfied');
      if (listed.length) fail('unknown');
      if (!applying) return reply('absent');
      const result = await api('hosting', `${cp.version}:populateFiles`, 'POST', {
        files: Object.fromEntries(expected.map((f) => [f.path, f.hash])),
      });
      const expectedUrl = `https://upload-firebasehosting.googleapis.com/upload/${cp.version}/files`;
      if (
        result.uploadUrl !== expectedUrl ||
        !Array.isArray(result.uploadRequiredHashes) ||
        result.uploadRequiredHashes.some((hash) => !files.some((f) => f.hash === hash))
      )
        fail('unknown');
      return reply('done');
    }
    if (i.action === 'upload') {
      const listed = await versionFiles();
      const matching = listed.filter((f) => f.hash === i.hash);
      if (
        !matching.length ||
        matching.some((f) => !files.some((e) => e.path === f.path && e.hash === f.hash))
      )
        fail('unknown');
      if (matching.every((f) => f.status === 'ACTIVE'))
        return reply(applying ? 'done' : 'satisfied');
      if (!applying) return reply('absent');
      const bytes = Buffer.from(i.bytesBase64, 'base64');
      if (
        bytes.length > 4 * 1024 * 1024 + 65536 ||
        sha(bytes) !== i.hash ||
        bytes.toString('base64') !== i.bytesBase64
      )
        fail('provider-rejected');
      credential = await identity();
      effect = true;
      const result = await deps.http(
        `https://upload-firebasehosting.googleapis.com/upload/${cp.version}/files/${i.hash}`,
        {
          method: 'POST',
          headers: {
            Authorization: `Bearer ${credential.token}`,
            'Content-Type': 'application/octet-stream',
          },
          body: bytes,
        },
        deadline
      );
      if (result.status !== 200) fail(fault(result, result.body));
      return reply('done');
    }
    if (i.action === 'finalize') {
      if (version.status === 'FINALIZED') return reply(applying ? 'done' : 'satisfied');
      if (stable(await versionFiles()) !== stable(files)) return reply('building');
      if (!applying) return reply('absent');
      await api('hosting', `${cp.version}?updateMask=status`, 'PATCH', { status: 'FINALIZED' });
      return reply('done');
    }
    if (i.action === 'release') {
      if (await exactLive()) return reply(applying ? 'done' : 'satisfied');
      const live = (await liveHosting()).live;
      // A retained own release is still checked by its complete frozen fingerprint.
      const fingerprint = live === null ? null : sha(stable(live));
      if (h.replaces === 'foreign' && fingerprint !== h.replacedFingerprint)
        fail('provider-rejected');
      if (h.replaces === 'none' && live !== null) fail('provider-rejected');
      if (h.replaces === 'own' && live?.deploymentId !== i.deployment) fail('provider-rejected');
      if (!applying) return reply('absent');
      if (version.status !== 'FINALIZED' || stable(await versionFiles()) !== stable(files))
        fail('unknown');
      // Re-read immediately before the public switch; no cross-service CAS exists.
      if (stable((await liveHosting()).live) !== stable(live)) fail('unknown');
      await api(
        'hosting',
        `sites/${project}/releases?versionName=${encodeURIComponent(cp.version)}`,
        'POST',
        {}
      );
      if (!(await exactLive())) fail('unknown');
      return reply('done');
    }
    if (i.action === 'verify') {
      if (
        !(await selectedApp()) ||
        version.status !== 'FINALIZED' ||
        !(await exactLive()) ||
        (await liveRules()) !== i.source
      )
        fail('unknown');
      const config = await api(
        'firebase',
        `projects/${project}/webApps/${encodeURIComponent(cp.appId)}/config`
      );
      if (
        config.projectId !== project ||
        config.appId !== cp.appId ||
        config.authDomain !== `${project}.firebaseapp.com` ||
        typeof config.apiKey !== 'string'
      )
        fail('unknown');
      const release = (await api('hosting', `sites/${project}/channels/live`)).release;
      if (
        release.version?.name !== cp.version ||
        !(await exactLive()) ||
        (await liveRules()) !== i.source
      )
        fail('unknown');
      return {
        publicConfig: {
          apiKey: config.apiKey,
          authDomain: config.authDomain,
          projectId: project,
          appId: cp.appId,
        },
        release: resource(release.name, `sites/${project}/releases/`),
      };
    }
    fail('provider-rejected');
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
    if (['observe-hosting', 'apply-hosting', 'verify-hosting'].includes(r.operation))
      return await hosting();
    // Validate an existing Firebase project; this adapter never creates or enables one.
    const p = await api('firebase', `projects/${project}`);
    if (p.projectId !== project || p.state !== 'ACTIVE') fail('provider-rejected');
    if (r.operation === 'live-rules' || r.operation === 'observe-rules') return await liveRules();
    if (r.operation === 'index-budget') {
      // Plan-only inventory includes unrelated live configs, before any effect.
      const present = await api('firestore', database, 'GET', undefined, true);
      const configured =
        present === null
          ? []
          : await pages(
              'firestore',
              `${database}/collectionGroups/-/fields?filter=${encodeURIComponent('indexConfig.usesAncestorConfig=false OR ttlConfig:*')}`,
              'fields'
            );
      const unique = new Set(configured.map((x) => x.name));
      if (
        configured.some(
          (x) => typeof x.name !== 'string' || !x.name.startsWith(`${database}/collectionGroups/`)
        ) ||
        unique.size !== configured.length
      )
        fail('unknown');
      for (const field of i.fields) {
        const [collection, name] = field.split('/');
        unique.add(`${database}/collectionGroups/${collection}/fields/${name}`);
      }
      if (unique.size > 200) fail('quota-exceeded');
      return 'within-budget';
    }
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
    if (Buffer.byteLength(raw) > UPLOAD_BYTES) fail('provider-rejected');
    const request = validate(JSON.parse(raw));
    if (request.operation !== 'apply-hosting' && Buffer.byteLength(raw) > BYTES)
      fail('provider-rejected');
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
    if (size > UPLOAD_BYTES) {
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
