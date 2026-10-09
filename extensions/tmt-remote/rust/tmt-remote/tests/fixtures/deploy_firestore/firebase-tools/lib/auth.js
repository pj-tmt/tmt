// Test-only login and fixed Google responses. Never loads a real credential store.
const fs = require('node:fs');
const path = require('node:path');
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
  let result;
  if (url.endsWith('/oauth2/v3/userinfo')) result = { email: value.account, email_verified: true };
  else if (url.startsWith('https://firebase.googleapis.com/'))
    result = { projectId: value.project, state: 'ACTIVE' };
  else if (url.includes('/releases/'))
    result = { rulesetName: `projects/${value.project}/rulesets/exact` };
  else if (url.includes('/rulesets/'))
    result = { source: { files: [{ name: 'firestore.rules', content: value.source }] } };
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
