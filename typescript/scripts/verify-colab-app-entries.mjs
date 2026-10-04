#!/usr/bin/env node
// Checks a built Colab app directory against the declared app entries before any release:
// the same inventory read the archive proof makes, so an undeclared top-level file or a
// missing declared one fails the pull request that builds the app, not the release.
import assert from 'node:assert/strict';
import path from 'node:path';
import { expectedFiles } from './colab-runtime-proof.mjs';

const [directory, ...extra] = process.argv.slice(2);
assert(directory && !extra.length, 'Usage: verify-colab-app-entries.mjs <built app directory>');
const files = expectedFiles(path.resolve(directory));
console.log(`Colab app inventory matches its declared entries (${files.size} files).`);
