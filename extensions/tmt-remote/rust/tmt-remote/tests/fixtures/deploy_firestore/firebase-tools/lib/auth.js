// Test-only login and fixed Google responses. Never loads a real credential store.
const fs = require('node:fs');
const path = require('node:path');
// The helper must reach auth from its neutral cwd, never a caller project.
if (process.cwd() !== '/') throw new Error('TOKEN_CANARY_UNSAFE_CWD');
fs.writeFileSync(path.join(__dirname, '../auth-loaded'), 'loaded');
const state = () => JSON.parse(fs.readFileSync(path.join(__dirname, '../state.json')));
exports.selectAccount = () => {
  const value = state();
  if (value.mode === 'throw') {
    process.stdout.write(value.canary);
    process.stderr.write(value.canary);
    throw new Error(value.canary);
  }
  if (value.mode === 'missing') return undefined;
  return { user: { email: value.account }, tokens: { refresh_token: value.canary } };
};
exports.getAccessToken = async () => {
  const value = state();
  if (value.mode === 'hang') {
    const net = require('node:net');
    await new Promise(() => {
      const socket = net.createConnection(value.gate);
      socket.on('connect', () => socket.write(`${process.pid}\n`));
    });
  }
  return { access_token: value.canary };
};
global.fetch = async (url, options) => {
  const value = state();
  fs.appendFileSync(
    path.join(__dirname, '../calls.jsonl'),
    JSON.stringify({ url, method: options.method }) + '\n'
  );
  if (
    value.hosting &&
    (url.startsWith('https://firebasehosting.googleapis.com/') ||
      url.startsWith('https://upload-firebasehosting.googleapis.com/') ||
      url.includes('/webApps'))
  ) {
    const h = value.hosting;
    const route = new URL(url).pathname;
    const body = options.body && !Buffer.isBuffer(options.body) ? JSON.parse(options.body) : null;
    let result,
      status = 200;
    if (route.endsWith('/webApps')) {
      if (options.method === 'POST') {
        h.apps.push({
          name: `projects/${value.project}/webApps/1:123:web:mine`,
          appId: '1:123:web:mine',
          projectId: value.project,
          displayName: body.displayName,
          state: 'ACTIVE',
        });
        result = { name: 'operations/app-one', done: false };
      } else result = { apps: h.apps };
    } else if (url.includes('/webApps/') && route.endsWith('/config'))
      result = {
        projectId: value.project,
        appId: '1:123:web:mine',
        authDomain: `${value.project}.firebaseapp.com`,
        apiKey: 'public-api-key',
      };
    else if (route === `/v1beta1/projects/${value.project}/sites/${value.project}`) {
      if (options.method === 'PATCH') {
        if (
          new URL(url).searchParams.get('updateMask') !== 'appId' ||
          Object.keys(body).join() !== 'appId' ||
          h.site.appId
        )
          throw Error('TOKEN_CANARY_SITE_REPOINT');
        h.site.appId = body.appId;
      }
      result = h.site ?? {};
      if (!h.site) status = 404;
    } else if (route === '/v1beta1/operations/app-one') {
      result = {};
      status = 404;
    } else if (route === `/v1beta1/projects/${value.project}/sites`) {
      if (Object.keys(body).length) throw Error('TOKEN_CANARY_SITE_ASSOCIATION');
      h.site = {
        name: `projects/${value.project}/sites/${value.project}`,
        defaultUrl: `https://${value.project}.web.app`,
      };
      result = h.site;
    } else if (route === `/v1beta1/sites/${value.project}/versions`) {
      if (options.method === 'POST') {
        h.version = {
          name: `sites/${value.project}/versions/one`,
          createTime: new Date().toISOString(),
          status: 'CREATED',
          ...body,
        };
        result = h.version;
      } else result = { versions: h.version ? [h.version] : [] };
    } else if (route === `/v1beta1/sites/${value.project}/versions/one`) {
      if (options.method === 'PATCH') h.version.status = body.status;
      result = h.version;
    } else if (route.endsWith('/versions/one:populateFiles')) {
      h.files = Object.entries(body.files).map(([path, hash]) => ({
        path,
        hash,
        status: 'EXPECTED',
      }));
      result = {
        uploadRequiredHashes: h.files.map((f) => f.hash),
        uploadUrl: `https://upload-firebasehosting.googleapis.com/upload/sites/${value.project}/versions/one/files`,
      };
    } else if (url.startsWith('https://upload-firebasehosting.googleapis.com/')) {
      h.files.forEach((f) => (f.status = 'ACTIVE'));
      result = null;
    } else if (route.endsWith('/versions/one/files')) result = { files: h.files };
    else if (route.endsWith('/channels/live')) result = h.live ? { release: h.live } : {};
    else if (route === `/v1beta1/sites/${value.project}/releases`) {
      h.live = { name: `sites/${value.project}/releases/one`, version: h.version };
      result = h.live;
    } else throw Error('TOKEN_CANARY_UNEXPECTED_HOSTING_ROUTE');
    fs.writeFileSync(path.join(__dirname, '../state.json'), JSON.stringify(value));
    return new Response(result === null ? null : JSON.stringify(result), { status });
  }
  // Binary fixtures retain the default release across separate helper processes.
  if (value.mode === 'process' && url.startsWith('https://firebaserules.googleapis.com/')) {
    const name = `projects/${value.project}/rulesets/exact`;
    const save = () =>
      fs.writeFileSync(path.join(__dirname, '../state.json'), JSON.stringify(value));
    let body,
      status = 200;
    if (url.includes('/rulesets?')) body = { rulesets: value.created ? [{ name }] : [] };
    else if (url.endsWith('/rulesets') && options.method === 'POST') {
      value.source = JSON.parse(options.body).source.files[0].content;
      value.created = true;
      save();
      body = { name };
    } else if (url.endsWith('/releases') && options.method === 'POST') {
      value.released = true;
      save();
      body = JSON.parse(options.body);
    } else if (url.includes('/releases/')) {
      if (!value.released) {
        status = 404;
        body = {};
      } else body = { rulesetName: name };
    } else if (url.includes('/rulesets/'))
      body = { source: { files: [{ name: 'firestore.rules', content: value.source }] } };
    else throw new Error('TOKEN_CANARY_UNEXPECTED_FIXTURE_ROUTE');
    return new Response(JSON.stringify(body), { status });
  }
  let result;
  if (url.endsWith('/oauth2/v3/userinfo')) result = { email: value.account, email_verified: true };
  else if (url.startsWith('https://firebase.googleapis.com/'))
    result = { projectId: value.project, state: 'ACTIVE' };
  else if (url.includes('/releases/'))
    result = { rulesetName: `projects/${value.project}/rulesets/exact` };
  else if (url.includes('/rulesets/'))
    result = { source: { files: [{ name: 'firestore.rules', content: value.source }] } };
  else if (url.includes('/collectionGroups/-/fields?')) result = { fields: value.configs ?? [] };
  else if (url.includes('/fields/')) {
    const name = new URL(url).pathname.slice(4),
      field = name.split('/').at(-1);
    result = {
      name,
      indexConfig: {
        indexes: ['ASCENDING', 'DESCENDING'].map((order) => ({
          queryScope: 'COLLECTION',
          state: 'READY',
          fields: [{ fieldPath: field, order }],
        })),
      },
    };
  } else if (url.endsWith('/config')) result = { signIn: { anonymous: { enabled: true } } };
  else
    result = {
      name: `projects/${value.project}/databases/(default)`,
      locationId: value.location,
      type: 'FIRESTORE_NATIVE',
      databaseEdition: 'STANDARD',
      createTime: '2026-01-01T00:00:00Z',
    };
  return new Response(JSON.stringify(result), { status: 200 });
};
