#!/usr/bin/env node
// Deterministic public-core peer. Remote remains the real signed/admission runtime.
import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { appendFileSync, existsSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const root = process.env.TMT_FIXTURE_ROOT;
assert.ok(root);
const receiptPath = join(root, 'receipt.json');
const agentId = '00000000-0000-0000-0000-000000000001';
function output(value) {
  process.stdout.write(JSON.stringify(value));
}
if (process.argv[2] === 'list') {
  assert.deepEqual(process.argv.slice(2), ['list', '--json']);
  output({
    identities: [
      { id: agentId, name: 'Browser agent', presence: 'active', pane: '%private', cwd: '/private' },
    ],
  });
} else {
  assert.deepEqual(process.argv.slice(2), ['api']);
  const request = JSON.parse(readFileSync(0, 'utf8'));
  appendFileSync(join(root, 'core-calls.jsonl'), `${JSON.stringify(request)}\n`);
  assert.equal(request.version, 1);
  switch (request.operation) {
    case 'capabilities':
      output({ version: 1, limits: { inputBytes: 1024, outputBytes: 4096 } });
      break;
    case 'storage.root':
      output({ dataRoot: join(root, 'state') });
      break;
    case 'dispatch.show':
      if (existsSync(receiptPath)) {
        const receipt = JSON.parse(readFileSync(receiptPath, 'utf8'));
        assert.equal(receipt.operationId, request.input.operationId);
        output(receipt);
      } else {
        output({ error: { code: 'DISPATCH_NOT_FOUND', message: 'No dispatch.' } });
        process.exitCode = 3;
      }
      break;
    case 'dispatch.create': {
      assert.equal(existsSync(receiptPath), false, 'Only one core dispatch may be created.');
      assert.equal(request.originator, 'anonymous');
      assert.equal(request.input.kind, 'request');
      assert.deepEqual(request.input.recipientIds, [agentId]);
      const receipt = {
        operationId: request.input.operationId,
        items: [{ recipientId: agentId, requestId: `req_${randomUUID()}`, acceptance: 'queued' }],
      };
      writeFileSync(receiptPath, JSON.stringify(receipt));
      writeFileSync(join(root, 'dispatched-message.txt'), request.input.message);
      output(receipt);
      break;
    }
    case 'requests.show': {
      const receipt = JSON.parse(readFileSync(receiptPath, 'utf8'));
      assert.equal(request.input.requestId, receipt.items[0].requestId);
      output({
        requestId: request.input.requestId,
        final: { status: 'retained', response: 'Browser retained final 🎯' },
      });
      break;
    }
    default:
      throw new Error(`Unsupported fixture operation ${request.operation}`);
  }
}
