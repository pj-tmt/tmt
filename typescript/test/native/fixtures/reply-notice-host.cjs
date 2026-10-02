// Controlled protocol evidence for native process tests; never invokes tmux.
const fs = require('node:fs');
const path = require('node:path');
const root = process.env.TMT_954_ROOT;
const args = process.argv.slice(2);
// Prepare the exact executable without protocol evidence or transport effects.
if (args.length === 1 && args[0] === '__tmt_fixture_ready') {
  fs.writeFileSync(path.join(root, 'host-ready'), 'ready');
  process.exit(0);
}
const known = [
  'list-panes',
  'list-clients',
  'set-buffer',
  'paste-buffer',
  'send-keys',
  'delete-buffer',
];
const command = args.find((arg) => known.includes(arg));
fs.appendFileSync(path.join(root, 'host-commands.jsonl'), JSON.stringify(args) + '\n');
if (command === 'list-panes') {
  console.log(fs.readFileSync(path.join(root, 'endpoint'), 'utf8'));
} else if (command === 'list-clients') {
  const pending = fs.readFileSync(path.join(root, 'typing'), 'utf8') === 'pending';
  const activity = pending ? Math.floor(Date.now() / 1000) : 1;
  fs.writeSync(1, '%1__TMT_FIELD_4f1c__' + activity + '\n');
  fs.appendFileSync(
    path.join(root, 'key-evidence.jsonl'),
    JSON.stringify({ pending, activity }) + '\n'
  );
} else if (!known.includes(command)) {
  process.exit(97);
}
