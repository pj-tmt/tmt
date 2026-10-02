// Controlled protocol evidence for native process tests; never invokes tmux.
const fs = require('node:fs');
const path = require('node:path');
const root = process.env.TMT_954_ROOT;
const args = process.argv.slice(2);
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
  console.log('%1__TMT_FIELD_4f1c__' + (pending ? Math.floor(Date.now() / 1000) : 1));
} else if (!known.includes(command)) {
  process.exit(97);
}
