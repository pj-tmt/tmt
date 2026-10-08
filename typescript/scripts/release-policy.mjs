#!/usr/bin/env node
import { parseArgs } from 'node:util';
import { checkLatestTag, releaseFlags, releasePolicy } from './native-release-policy.mjs';

const { values } = parseArgs({
  options: {
    product: { type: 'string' },
    'check-latest': { type: 'string' },
  },
});
if (values['check-latest'] !== undefined) {
  checkLatestTag(values['check-latest']);
  process.stdout.write(`${values['check-latest']} is a CLI release.\n`);
} else if (values.product !== undefined) {
  process.stdout.write(
    `${JSON.stringify({ ...releasePolicy(values.product), flags: releaseFlags(values.product) })}\n`
  );
} else {
  throw new Error('Usage: --product <cli|office|ops|driver-herdr|colab> | --check-latest <tag>');
}
