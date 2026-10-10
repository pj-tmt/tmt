import assert from 'node:assert/strict';
import { test } from 'vite-plus/test';
import { openSession } from '../src/device.js';
import { ClientError, RefusalError, operations, type ClientErrorCode } from '../src/operations.js';
import { Door, paired } from './door.js';

async function ready(door = new Door(), timeoutMs?: number) {
  const device = await paired(door);
  const session = await openSession(
    device.result,
    device.key,
    door.descriptor.windowId,
    door.fetch,
  );
  return { door, device, session, client: operations(session, { timeoutMs }) };
}
const intent = () => ({
  operationId: crypto.randomUUID(),
  agentId: crypto.randomUUID(),
  message: 'Ask!\nExact e\u0301 🎯\u0000',
});
const request = () => `req_${crypto.randomUUID()}`;
const unknown =
  (code: ClientErrorCode) =>
  (error: unknown): boolean =>
    error instanceof ClientError && error.code === code;
function barrier() {
  let resolve!: () => void;
  const promise = new Promise<void>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

test('constructing operations opens nothing; all four calls share one verified lane and freeze intent', async () => {
  const { door, session, client } = await ready();
  const value = intent();
  const original = { ...value };
  const sending = client.send(value);
  value.message = 'changed after send';
  const [sent, agents] = await Promise.all([sending, operations(session).listAgents()]);
  assert.equal(door.opens, 1);
  assert.equal(sent.state, 'accepted');
  assert.deepEqual(agents, door.agents);
  assert.equal(Object.hasOwn(agents[0]!, 'delivery'), false);
  assert.equal(door.calls[0]!.envelope.id, original.operationId);
  assert.deepEqual(door.calls[0]!.payload, {
    version: 1,
    operation: 'dispatch.create',
    originator: 'anonymous',
    input: {
      operationId: original.operationId,
      recipientIds: [original.agentId],
      message: original.message,
      kind: 'request',
    },
  });
  assert.deepEqual(await client.operation(original.operationId), sent);
  assert.ok(sent.state === 'accepted');
  assert.deepEqual(await client.result(sent.requestId), {
    state: 'replied',
    requestId: sent.requestId,
    message: '',
  });
  assert.deepEqual(
    door.calls.map((call) => call.envelope.sequence),
    ['1', '2', '3', '4'],
  );
  assert.equal(
    door.calls.filter((call) => call.envelope.operation === 'dispatch.create').length,
    1,
  );
});
for (const state of ['held', 'accepted', 'uncertain', 'refused', 'cancelled']) {
  test(`send and operation preserve signed ${state} state without retry`, async () => {
    const { door, client } = await ready();
    door.sendState = state;
    const value = intent();
    const sent = await client.send(value);
    assert.equal(sent.state, state);
    assert.equal(sent.operationId, value.operationId);
    assert.deepEqual(await client.operation(value.operationId), sent);
    assert.deepEqual(
      door.calls.map((call) => call.envelope.operation),
      ['dispatch.create', 'operation.show'],
    );
  });
}
for (const state of ['pending', 'replied', 'unavailable']) {
  test(`result preserves ${state} and exact inert final text`, async () => {
    const { door, client } = await ready();
    door.replyState = state;
    door.final = '<script>no execution</script>\n🎯';
    const id = request();
    assert.deepEqual(await client.result(id), {
      state,
      requestId: id,
      ...(state === 'replied' ? { message: door.final } : {}),
    });
  });
}

test('agents forward optional delivery unchanged and tolerate additive data', async () => {
  const { door, client } = await ready();
  door.agents = [
    {
      id: crypto.randomUUID(),
      name: 'Ready',
      presence: 'active',
      delivery: { state: 'channel', hint: 'core-owned' },
      extra: 'additive',
    },
  ];
  door.mutate = (reply) => {
    reply.extra = 'additive';
  };
  const listed = await client.listAgents();
  assert.deepEqual(
    listed,
    door.agents.map((value) => {
      const { extra: _, ...agent } = value as Record<string, unknown>;
      return agent;
    }),
  );
});

test('agents decode only recognized runningDriver and never remembered driver', async () => {
  const { door, client } = await ready();
  const evidence = ['claude', 'codex', undefined, 'future-driver', 'CLAUDE', null, 7, {}];
  door.agents = evidence.map((runningDriver, index) => ({
    id: crypto.randomUUID(),
    name: 'Codex remembered label',
    presence: index === 2 ? 'offline' : 'active',
    driver: 'codex',
    ...(runningDriver === undefined ? {} : { runningDriver }),
    delivery: { state: 'not_running' },
  }));
  const listed = await client.listAgents();
  assert.equal(listed.length, evidence.length);
  for (const [index, row] of listed.entries()) {
    const runningDriver = evidence[index];
    assert.equal(Object.hasOwn(row, 'runningDriver'), index < 2);
    assert.deepEqual(row, {
      id: (door.agents[index] as { id: string }).id,
      name: 'Codex remembered label',
      presence: index === 2 ? 'offline' : 'active',
      delivery: { state: 'not_running' },
      ...(index < 2 ? { runningDriver } : {}),
    });
  }
  assert.deepEqual(
    door.calls.map((call) => call.envelope.operation),
    ['agents.list'],
  );
});

for (const code of [
  'REMOTE_SCOPE_DENIED',
  'REMOTE_INPUT_INVALID',
  'REMOTE_RATE_LIMITED',
  'REMOTE_INTENT_CONFLICT',
  'REMOTE_CLOSED',
]) {
  for (const method of ['send', 'operation'] as const) {
    test(`${method} returns verified pre-effect ${code} as a refused state`, async () => {
      const { door, client } = await ready();
      door.error = { code, message: '/private/raw-output must not leak', retryAfterMs: 123 };
      door.errorBeforeConsume = true;
      const value = intent();
      const state = await (method === 'send'
        ? client.send(value)
        : client.operation(value.operationId));
      assert.deepEqual(state, { state: 'refused', operationId: value.operationId, reason: code });
      assert.equal(door.intents.size, 0);
      if (code !== 'REMOTE_CLOSED') {
        door.error = undefined;
        assert.deepEqual(await client.listAgents(), door.agents);
        assert.deepEqual(
          door.calls.map((call) => call.envelope.sequence),
          ['1', '2', '1', '2'],
        );
      }
    });
  }
}
for (const method of ['send', 'operation'] as const) {
  test(`${method} treats pre-admission 404 as session ended without dispatch`, async () => {
    const door = new Door();
    const device = await paired(door);
    let ended = false;
    const send = ((...args: Parameters<typeof fetch>) =>
      ended
        ? Promise.resolve(new Response('{}', { status: 404 }))
        : door.fetch(...args)) as typeof fetch;
    const session = await openSession(device.result, device.key, door.descriptor.windowId, send);
    const remote = operations(session);
    ended = true;
    const value = intent();
    assert.deepEqual(
      await (method === 'send' ? remote.send(value) : remote.operation(value.operationId)),
      { state: 'refused', operationId: value.operationId, reason: 'REMOTE_SESSION_ENDED' },
    );
    assert.equal(door.intents.size, 0);
    ended = false;
    assert.deepEqual(await remote.operation(value.operationId), {
      state: 'refused',
      operationId: value.operationId,
      reason: 'REMOTE_SESSION_ENDED',
    });
    assert.equal(door.opens, 1);
    assert.equal(door.calls.length, 0);
  });
}
for (const method of ['listAgents', 'result'] as const) {
  test(`${method} exposes typed RefusalError for signed refusal and bounded retry hint`, async () => {
    const { door, client } = await ready();
    door.error = { code: 'REMOTE_RATE_LIMITED', message: 'private raw detail', retryAfterMs: 123 };
    await assert.rejects(
      method === 'listAgents' ? client.listAgents() : client.result(request()),
      (error) =>
        error instanceof RefusalError &&
        !(error instanceof ClientError) &&
        error.code === 'REMOTE_RATE_LIMITED' &&
        error.retryAfterMs === 123 &&
        !error.message.includes('private'),
    );
  });
}

const changes: Record<string, (reply: Record<string, unknown>) => void> = {
  signature: (reply) => {
    reply.signature = 'A'.repeat(86);
  },
  correlation: (reply) => {
    reply.correlationId = crypto.randomUUID();
  },
  machine: (reply) => {
    reply.machineId = crypto.randomUUID();
  },
  window: (reply) => {
    reply.windowId = crypto.randomUUID();
  },
  client: (reply) => {
    reply.clientId = crypto.randomUUID();
  },
  session: (reply) => {
    reply.sessionId = crypto.randomUUID();
  },
  origin: (reply) => {
    reply.origin = 'cli';
  },
  operation: (reply) => {
    reply.operation = 'operation.show';
  },
  sequence: (reply) => {
    reply.sequence = '1';
  },
  payload: (reply) => {
    reply.payload = Buffer.from('{ "identities": [] }').toString('base64url');
  },
  base64: (reply) => {
    reply.payload = `${String(reply.payload)}=`;
  },
};
for (const [name, change] of Object.entries(changes)) {
  test(`unverifiable ${name} cannot expose data; recovery keeps the same session`, async () => {
    const { door, client } = await ready();
    assert.deepEqual(await client.listAgents(), door.agents);
    if (['signature', 'payload', 'base64'].includes(name)) door.afterSign = change;
    else door.mutate = change;
    await assert.rejects(client.listAgents(), unknown('unverifiable_response'));
    door.afterSign = undefined;
    door.mutate = undefined;
    assert.deepEqual(await client.listAgents(), door.agents);
    assert.equal(door.opens, 1);
  });
}

test('correctly signed foreign state IDs and malformed accepted shapes are unknown outcomes', async () => {
  for (const shape of ['foreign-id', 'bad-request', 'unknown-state']) {
    const { door, client } = await ready();
    const value = intent();
    door.mutate = (reply) => {
      reply.payload = Buffer.from(
        JSON.stringify({
          state: shape === 'unknown-state' ? 'mystery' : 'accepted',
          operationId: shape === 'foreign-id' ? crypto.randomUUID() : value.operationId,
          requestId: shape === 'bad-request' ? 'invalid' : request(),
        }),
      ).toString('base64url');
    };
    await assert.rejects(
      client.send(value),
      (error) =>
        unknown('unverifiable_response')(error) &&
        (error as ClientError).operationId === value.operationId,
    );
  }
});
for (const mode of ['lost', 'timeout', 'unsigned-status'] as const) {
  test(`${mode} after dispatch adoption throws ClientError and recovers without reopening`, async () => {
    const { door, client } = await ready(undefined, mode === 'timeout' ? 100 : undefined);
    const entered = barrier();
    const release = barrier();
    door.afterAdoption = async (body, init) => {
      if (body.operation !== 'dispatch.create') return;
      entered.resolve();
      if (mode === 'timeout') {
        init.signal!.addEventListener('abort', release.resolve, { once: true });
        await release.promise;
      } else if (mode === 'lost') throw new Error('lost after adoption');
    };
    if (mode === 'unsigned-status') door.httpStatus = 503;
    const value = intent();
    const rejected = assert.rejects(
      client.send(value),
      (error) =>
        unknown(mode === 'timeout' ? 'timeout' : 'transport_failure')(error) &&
        (error as ClientError).operationId === value.operationId,
    );
    await entered.promise;
    await rejected;
    door.afterAdoption = undefined;
    door.httpStatus = 200;
    assert.deepEqual(
      await client.operation(value.operationId),
      door.receipts.get(value.operationId),
    );
    assert.deepEqual(
      door.calls.map((call) => call.envelope.operation),
      ['dispatch.create', 'capabilities', 'operation.show'],
    );
    assert.equal(door.opens, 1);
    assert.equal(door.intents.size, 1);
  });
}

test('a dropped request before consumption uses two capabilities probes then observes with no dispatch replay', async () => {
  const door = new Door();
  const device = await paired(door);
  let drop = true;
  const send = ((...args: Parameters<typeof fetch>) => {
    const body = JSON.parse((args[1]?.body as string) ?? '{}') as { operation: string };
    if (body.operation === 'dispatch.create' && drop) {
      drop = false;
      throw new Error('Dropped before admission.');
    }
    return door.fetch(...args);
  }) as typeof fetch;
  const session = await openSession(device.result, device.key, door.descriptor.windowId, send);
  const client = operations(session);
  const value = intent();
  await assert.rejects(client.send(value), unknown('transport_failure'));
  // Capabilities synchronize the sequence; the subsequent missing-state read
  // confirms absence and never invites an automatic replacement dispatch.
  await assert.rejects(
    client.operation(value.operationId),
    (error) => error instanceof RefusalError && error.code === 'REMOTE_STATE_UNAVAILABLE',
  );
  assert.deepEqual(
    door.calls.map((call) => call.envelope.sequence),
    ['2', '1', '2'],
  );
  assert.deepEqual(
    door.calls.map((call) => call.envelope.operation),
    ['capabilities', 'capabilities', 'operation.show'],
  );
  assert.equal(door.calls[0]!.envelope.id, door.calls[1]!.envelope.id);
  assert.equal(door.calls[0]!.bytes, door.calls[1]!.bytes);
  assert.equal(door.opens, 1);
  assert.equal(door.intents.size, 0);
});

test('two replay refusals stop resync; no third guess until caller explicitly reopens', async () => {
  const { door, device, client } = await ready();
  door.afterAdoption = async () => {
    throw new Error('Lost');
  };
  const value = intent();
  await assert.rejects(client.send(value), unknown('transport_failure'));
  door.afterAdoption = undefined;
  door.forceReplay = true;
  await assert.rejects(client.operation(value.operationId), unknown('sequence_unavailable'));
  await assert.rejects(client.operation(value.operationId), unknown('sequence_unavailable'));
  assert.deepEqual(
    door.calls.map((call) => call.envelope.sequence),
    ['1', '2', '1'],
  );
  assert.equal(door.opens, 1);
  door.forceReplay = false;
  const session = await openSession(
    device.result,
    device.key,
    door.descriptor.windowId,
    door.fetch,
  );
  assert.deepEqual(
    await operations(session).operation(value.operationId),
    door.receipts.get(value.operationId),
  );
  assert.equal(door.intents.size, 1);
});

for (const method of ['operation', 'result', 'listAgents'] as const) {
  test(`${method} timeout recovers on the existing session without another dispatch`, async () => {
    const { door, client } = await ready(undefined, 100);
    const value = intent();
    const sent = await client.send(value);
    assert.ok(sent.state === 'accepted');
    const entered = barrier();
    const release = barrier();
    door.afterAdoption = async (_, init) => {
      entered.resolve();
      init.signal!.addEventListener('abort', release.resolve, { once: true });
      await release.promise;
    };
    const observation =
      method === 'operation'
        ? client.operation(value.operationId)
        : method === 'result'
          ? client.result(sent.requestId)
          : client.listAgents();
    const rejected = assert.rejects(observation, unknown('timeout'));
    await entered.promise;
    await rejected;
    door.afterAdoption = undefined;
    assert.deepEqual(await client.operation(value.operationId), sent);
    assert.equal(door.opens, 1);
    assert.equal(
      door.calls.filter((call) => call.envelope.operation === 'dispatch.create').length,
      1,
    );
  });
}

test('an explicit identical resend reconstructs identical payload bytes and receipt', async () => {
  const { door, client } = await ready();
  const value = intent();
  const first = await client.send(value);
  assert.deepEqual(await client.send({ ...value }), first);
  assert.equal(door.calls[0]!.bytes, door.calls[1]!.bytes);
  assert.equal(door.calls[0]!.envelope.id, door.calls[1]!.envelope.id);
});

test('invalid input and copied sessions never publish or throw an unknown-outcome ClientError', async () => {
  const { door, session, client } = await ready();
  assert.throws(() => operations({ ...session }), TypeError);
  for (const timeoutMs of [0, -1, NaN, 2147483648])
    assert.throws(() => operations(session, { timeoutMs }), TypeError);
  for (const change of [
    { operationId: 'wrong' },
    { agentId: '00000000-0000-0000-0000-000000000000' },
    { message: '\ud800' },
  ])
    await assert.rejects(client.send({ ...intent(), ...change }), TypeError);
  await assert.rejects(client.operation('wrong'), TypeError);
  await assert.rejects(client.result('wrong'), TypeError);
  assert.equal(door.calls.length, 0);
});

test('native fetch is invoked without the channel as its receiver', async () => {
  const door = new Door();
  const device = await paired(door);
  const send = function (
    this: undefined,
    ...args: Parameters<typeof fetch>
  ): ReturnType<typeof fetch> {
    assert.equal(this, undefined);
    return door.fetch(...args);
  } as typeof fetch;
  const session = await openSession(device.result, device.key, door.descriptor.windowId, send);
  assert.deepEqual(await operations(session).listAgents(), door.agents);
});

test('a queued explicit identical send synchronizes with capabilities before dispatching', async () => {
  const { door, session, client } = await ready();
  let drop = true;
  door.afterAdoption = async (body) => {
    if (drop && body.operation === 'dispatch.create') {
      drop = false;
      throw new Error('Lost');
    }
  };
  const first = intent();
  const sending = assert.rejects(client.send(first), unknown('transport_failure'));
  const queued = operations(session).send({ ...first });
  const observing = client.operation(first.operationId);
  await sending;
  assert.deepEqual(await queued, door.receipts.get(first.operationId));
  assert.deepEqual(await observing, door.receipts.get(first.operationId));
  assert.equal(door.opens, 1);
  assert.equal(door.intents.size, 1);
  assert.deepEqual(
    door.calls.map((call) => call.envelope.operation),
    ['dispatch.create', 'capabilities', 'dispatch.create', 'operation.show'],
  );
  assert.equal(door.calls[0]!.bytes, door.calls[2]!.bytes);
});

test('a lost capabilities resync response stops further guesses without closing or reopening the server session', async () => {
  const { door, client } = await ready();
  door.afterAdoption = async () => {
    throw new Error('Lost');
  };
  const value = intent();
  await assert.rejects(client.send(value), unknown('transport_failure'));
  await assert.rejects(client.operation(value.operationId), unknown('sequence_unavailable'));
  door.afterAdoption = undefined;
  await assert.rejects(client.listAgents(), unknown('sequence_unavailable'));
  assert.deepEqual(
    door.calls.map((call) => call.envelope.sequence),
    ['1', '2'],
  );
  assert.equal(door.opens, 1);
});

for (const code of [
  'REMOTE_INPUT_TOO_LARGE',
  'REMOTE_STATE_UNAVAILABLE',
  'REMOTE_CORE_UNAVAILABLE',
]) {
  test(`signed ${code} on reads is a RefusalError`, async () => {
    const { door, client } = await ready();
    door.error = { code, message: 'private detail' };
    await assert.rejects(
      client.listAgents(),
      (error) => error instanceof RefusalError && error.code === code,
    );
    door.error = undefined;
    assert.deepEqual(await client.listAgents(), door.agents);
    assert.deepEqual(
      door.calls.map((call) => call.envelope.sequence),
      ['1', '2', '3'],
    );
  });
}

for (const code of ['REMOTE_STATE_UNAVAILABLE', 'REMOTE_CORE_UNAVAILABLE']) {
  test(`send never converts signed ${code} to refused; adopted uncertainty stays uncertain`, async () => {
    const { door, client } = await ready();
    door.error = { code, message: 'No proven pre-effect classification.' };
    const first = intent();
    await assert.rejects(
      client.send(first),
      (error) => error instanceof RefusalError && error.code === code,
    );
    assert.equal(door.intents.size, 0);
    door.error = undefined;
    // Observe the original ID before any caller-owned explicit send. The read
    // can prove absence but never authorizes an automatic replacement dispatch.
    await assert.rejects(
      client.operation(first.operationId),
      (error) => error instanceof RefusalError && error.code === 'REMOTE_STATE_UNAVAILABLE',
    );
    assert.deepEqual(await client.listAgents(), door.agents);
    door.sendState = 'uncertain';
    const second = intent();
    const expected = { state: 'uncertain', operationId: second.operationId };
    assert.deepEqual(await client.send(second), expected);
    assert.deepEqual(await client.operation(second.operationId), expected);
    assert.equal(door.intents.size, 1);
    assert.equal(door.opens, 1);
  });
}

test('a forged pre-effect refusal remains an unknown send outcome', async () => {
  const { door, client } = await ready();
  door.error = { code: 'REMOTE_SCOPE_DENIED', message: 'Untrusted refusal.' };
  door.afterSign = (reply) => {
    reply.signature = 'A'.repeat(86);
  };
  await assert.rejects(client.send(intent()), unknown('unverifiable_response'));
});

for (const beforeConsume of [true, false]) {
  test(`initial agents refusal resynchronizes internally without an operation ID (pre-consume=${beforeConsume})`, async () => {
    const { door, client } = await ready();
    door.error = { code: 'REMOTE_RATE_LIMITED', message: 'Rate refused.' };
    door.errorBeforeConsume = beforeConsume;
    await assert.rejects(
      client.listAgents(),
      (error) => error instanceof RefusalError && error.code === 'REMOTE_RATE_LIMITED',
    );
    door.error = undefined;
    assert.deepEqual(await client.listAgents(), door.agents);
    assert.deepEqual(
      door.calls.map((call) => call.envelope.operation),
      beforeConsume
        ? ['agents.list', 'capabilities', 'capabilities', 'agents.list']
        : ['agents.list', 'capabilities', 'agents.list'],
    );
    assert.equal(door.opens, 1);
    assert.equal(door.intents.size, 0);
  });
}

test('session-ended during capabilities resync remains a distinct refusal and never dispatches', async () => {
  const { door, client } = await ready();
  door.afterAdoption = async () => {
    throw new Error('Lost');
  };
  const value = intent();
  await assert.rejects(client.send(value), unknown('transport_failure'));
  door.afterAdoption = undefined;
  door.error = { code: 'REMOTE_CLOSED', message: 'Session ended.' };
  assert.deepEqual(await client.operation(value.operationId), {
    state: 'refused',
    operationId: value.operationId,
    reason: 'REMOTE_CLOSED',
  });
  await assert.rejects(
    client.listAgents(),
    (error) => error instanceof RefusalError && error.code === 'REMOTE_SESSION_ENDED',
  );
  assert.deepEqual(
    door.calls.map((call) => call.envelope.operation),
    ['dispatch.create', 'capabilities'],
  );
  assert.equal(door.opens, 1);
});

for (const code of ['REMOTE_SESSION_ENDED', 'REMOTE_SESSION_EVICTED'] as const) {
  test(`${code} is a typed, reopenable end without automatic send retry`, async () => {
    const { door, device, client } = await ready();
    door.error = {
      code,
      message: 'Session ended; reopen.',
      ...(code === 'REMOTE_SESSION_EVICTED' ? { limit: 8 } : {}),
    };
    await assert.rejects(
      client.listAgents(),
      (error: unknown) => error instanceof RefusalError && error.code === code,
    );
    assert.equal(door.opens, 1);
    const count = door.calls.length;
    await assert.rejects(
      client.listAgents(),
      (error: unknown) => error instanceof RefusalError && error.code === 'REMOTE_SESSION_ENDED',
    );
    assert.equal(door.calls.length, count);
    door.error = undefined;
    const reopened = await openSession(
      device.result,
      device.key,
      door.descriptor.windowId,
      door.fetch,
    );
    assert.deepEqual(await operations(reopened).listAgents(), door.agents);
    assert.equal(door.opens, 2);
  });
}
test('transportUrl binds only a verified session to a mounted transport URL', async () => {
  const { transportUrl } = await import('../src/browser.js');
  const { session, device } = await ready();
  const url = device.result.address.replace('http:', 'ws:') + '/x/colab/sync';
  assert.equal(transportUrl(session, url), `${url}?tmt-session=${session.sessionId}`);
  assert.throws(() => transportUrl({ ...session }, url), RefusalError);
  for (const invalid of [
    url.replace('ws:', 'http:'),
    url.replace('/x/colab/', '/append/'),
    `${url}?secret=1`,
    `${url}#fragment`,
    'ws://127.0.0.1:1/x/colab/sync',
  ]) {
    assert.throws(() => transportUrl(session, invalid), TypeError);
  }
});

test('transportUrl matches the paired door scheme for both http and https', async () => {
  const { transportUrl } = await import('../src/browser.js');
  for (const scheme of ['http:', 'https:']) {
    const { door, device } = await ready();
    const paired = { ...device.result, address: device.result.address.replace('http:', scheme) };
    const session = await openSession(paired, device.key, door.descriptor.windowId, door.fetch);
    const transport =
      paired.address.replace(scheme, scheme === 'https:' ? 'wss:' : 'ws:') + '/x/colab/sync';
    assert.equal(transportUrl(session, transport), `${transport}?tmt-session=${session.sessionId}`);
    assert.throws(
      () =>
        transportUrl(
          session,
          transport.replace(
            scheme === 'https:' ? 'wss:' : 'ws:',
            scheme === 'https:' ? 'ws:' : 'wss:',
          ),
        ),
      TypeError,
    );
  }
});
for (const settingsUrl of [undefined, null, '/remote-settings']) {
  test(`eviction exposes active limit and optional settings URL ${settingsUrl}`, async () => {
    const { door, client, device } = await ready();
    door.error = { code: 'REMOTE_SESSION_EVICTED', message: 'Evicted.', limit: 8, settingsUrl };
    await assert.rejects(
      client.listAgents(),
      (error: unknown) =>
        error instanceof RefusalError &&
        error.limit === 8 &&
        error.settingsUrl ===
          (settingsUrl ? new URL(settingsUrl, device.result.address).href : undefined),
    );
    const again = await ready();
    again.door.error = {
      code: 'REMOTE_SESSION_EVICTED',
      message: 'Evicted.',
      limit: 12,
      settingsUrl,
    };
    const result = await again.client.send(intent());
    assert.equal(result.state, 'refused');
    assert.ok(result.state === 'refused');
    assert.equal(result.limit, 12);
    assert.equal(
      result.settingsUrl,
      settingsUrl ? new URL(settingsUrl, again.device.result.address).href : undefined,
    );
  });
}
for (const bad of [
  { limit: 0 },
  { limit: -1 },
  { limit: 1.5 },
  { limit: 8, settingsUrl: 'javascript:alert(1)' },
  { limit: 8, settingsUrl: 'https://elsewhere.example/settings' },
]) {
  test(`eviction metadata rejects ${JSON.stringify(bad)}`, async () => {
    const { door, client } = await ready();
    door.error = { code: 'REMOTE_SESSION_EVICTED', message: 'Evicted.', ...bad };
    await assert.rejects(client.listAgents(), unknown('unverifiable_response'));
  });
}

for (const settingsUrl of [undefined, null, '/settings']) {
  test(`missing talk exposes its signed scope and optional settings URL ${settingsUrl}`, async () => {
    const { door, client, device } = await ready();
    door.error = {
      code: 'REMOTE_SCOPE_DENIED',
      message: 'Sending is not enabled.',
      scope: 'talk',
      settingsUrl,
    };
    await assert.rejects(
      client.listAgents(),
      (error: unknown) =>
        error instanceof RefusalError &&
        error.scope === 'talk' &&
        error.settingsUrl ===
          (settingsUrl ? new URL(settingsUrl, device.result.address).href : undefined),
    );
    const again = await ready();
    again.door.error = {
      code: 'REMOTE_SCOPE_DENIED',
      message: 'Sending is not enabled.',
      scope: 'talk',
      settingsUrl,
    };
    const result = await again.client.send(intent());
    assert.ok(result.state === 'refused');
    assert.equal(result.reason, 'REMOTE_SCOPE_DENIED');
    assert.equal(result.scope, 'talk');
  });
}
test('recipient scope denial does not claim the talk scope is missing', async () => {
  const { door, client } = await ready();
  door.error = { code: 'REMOTE_SCOPE_DENIED', message: 'Recipient is not permitted.' };
  await assert.rejects(
    client.listAgents(),
    (error: unknown) => error instanceof RefusalError && error.scope === undefined,
  );
});
