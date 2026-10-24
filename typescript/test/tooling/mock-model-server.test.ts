import { createConnection } from 'node:net';
import { once } from 'node:events';
import { afterEach, describe, expect, it } from 'vite-plus/test';
import {
  MOCK_MODEL_TOKEN,
  startMockModelServer,
  type ScriptedTextTurn,
} from '../support/mock-model-server.mjs';

type Server = Awaited<ReturnType<typeof startMockModelServer>>;
const servers: Server[] = [];
afterEach(async () => {
  await Promise.all(servers.splice(0).map((server) => server.close()));
});
const turn = (protocol: ScriptedTextTurn['protocol']): ScriptedTextTurn => ({
  protocol,
  model: 'fixture-model',
  text: 'Synthetic answer: 雪\nsecond line',
  inputTokens: 13,
  outputTokens: 7,
  cachedInputTokens: 3,
});
async function start(turns = [turn('messages')], limits = {}) {
  const server = await startMockModelServer({ turns, limits });
  servers.push(server);
  return server;
}
function body(protocol: ScriptedTextTurn['protocol']) {
  return {
    model: 'fixture-model',
    stream: true,
    ...(protocol === 'messages'
      ? { messages: [{ role: 'user', content: 'fixture input' }] }
      : { input: [{ role: 'user', content: 'fixture input' }] }),
  };
}
function post(
  server: Server,
  protocol: ScriptedTextTurn['protocol'],
  value: unknown = body(protocol)
) {
  return fetch(`${server.baseUrl}/v1/${protocol}`, {
    method: 'POST',
    headers: { authorization: `Bearer ${MOCK_MODEL_TOKEN}`, 'content-type': 'application/json' },
    body: JSON.stringify(value),
    signal: AbortSignal.timeout(2000),
  });
}
function events(wire: string) {
  expect(wire.endsWith('\n\n')).toBe(true);
  return wire
    .trimEnd()
    .split('\n\n')
    .map((frame) => {
      const lines = frame.split('\n');
      expect(lines).toHaveLength(2);
      expect(lines[0]).toMatch(/^event: /);
      expect(lines[1]).toMatch(/^data: /);
      const data = JSON.parse(lines[1].slice(6)) as Record<string, unknown>;
      expect(data.type).toBe(lines[0].slice(7));
      return data;
    });
}

describe('bounded loopback model fixture', () => {
  it('streams Messages text and cumulative usage with matching block identity', async () => {
    const server = await start();
    expect(server.baseUrl).toMatch(/^http:\/\/127\.0\.0\.1:\d+$/);
    const response = await post(server, 'messages');
    expect(response.status).toBe(200);
    expect(response.headers.get('content-type')).toBe('text/event-stream; charset=utf-8');
    const stream = events(await response.text());
    expect(stream.map((event) => event.type)).toEqual([
      'message_start',
      'content_block_start',
      'content_block_delta',
      'content_block_stop',
      'message_delta',
      'message_stop',
    ]);
    expect(stream[0]).toMatchObject({
      message: {
        id: 'msg_mock_1',
        model: 'fixture-model',
        role: 'assistant',
        content: [],
        stop_reason: null,
        usage: {
          input_tokens: 13,
          output_tokens: 0,
          cache_read_input_tokens: 3,
          cache_creation_input_tokens: 0,
        },
      },
    });
    expect(stream[1]).toMatchObject({ index: 0, content_block: { type: 'text', text: '' } });
    expect(stream[2]).toMatchObject({
      index: 0,
      delta: { type: 'text_delta', text: turn('messages').text },
    });
    expect(stream[3]).toMatchObject({ index: 0 });
    expect(stream[4]).toMatchObject({
      delta: { stop_reason: 'end_turn', stop_sequence: null },
      usage: { output_tokens: 7 },
    });
    expect(server.snapshot()).toMatchObject({
      completedTurns: 1,
      requestsSeen: 1,
      plannedTurns: 1,
    });
    expect(server.snapshot().failure).toBeUndefined();
  });

  it('streams Responses with ordered events, stable IDs, final output and non-double-counted usage', async () => {
    const server = await start([turn('responses')]);
    const response = await post(server, 'responses');
    expect(response.status).toBe(200);
    const stream = events(await response.text());
    expect(stream.map((event) => event.type)).toEqual([
      'response.created',
      'response.in_progress',
      'response.output_item.added',
      'response.content_part.added',
      'response.output_text.delta',
      'response.output_text.done',
      'response.content_part.done',
      'response.output_item.done',
      'response.completed',
    ]);
    expect(stream.map((event) => event.sequence_number)).toEqual([0, 1, 2, 3, 4, 5, 6, 7, 8]);
    expect(stream[0]).toMatchObject({
      response: { id: 'resp_mock_1', status: 'in_progress', output: [], usage: null },
    });
    expect(stream[2]).toMatchObject({
      output_index: 0,
      item: { id: 'msg_mock_1', status: 'in_progress', content: [] },
    });
    expect(stream[4]).toMatchObject({
      item_id: 'msg_mock_1',
      output_index: 0,
      content_index: 0,
      delta: turn('responses').text,
    });
    const item = {
      id: 'msg_mock_1',
      type: 'message',
      status: 'completed',
      role: 'assistant',
      content: [
        { type: 'output_text', text: turn('responses').text, annotations: [], logprobs: [] },
      ],
    };
    expect(stream[7]).toMatchObject({ item });
    expect(stream[8]).toMatchObject({
      response: {
        id: 'resp_mock_1',
        status: 'completed',
        output: [item],
        usage: {
          input_tokens: 13,
          output_tokens: 7,
          total_tokens: 20,
          input_tokens_details: { cached_tokens: 3 },
          output_tokens_details: { reasoning_tokens: 0 },
        },
      },
    });
    expect(server.snapshot().failure).toBeUndefined();
  });

  it('serves only the finite FIFO script, without treating readiness as a turn', async () => {
    const server = await start([turn('messages'), turn('responses')]);
    expect(await (await fetch(`${server.baseUrl}/healthz`)).json()).toEqual({ ready: true });
    expect(server.snapshot().completedTurns).toBe(0);
    await (await post(server, 'messages')).text();
    const second = events(await (await post(server, 'responses')).text());
    expect(second.at(-1)).toMatchObject({ response: { id: 'resp_mock_2' } });
    const extra = await post(server, 'responses');
    expect(extra.status).toBe(409);
    expect(await extra.json()).toMatchObject({ error: { code: 'MOCK_SCRIPT_EXHAUSTED' } });
    expect(server.snapshot().completedTurns).toBe(2);
    expect(server.snapshot().census.map((request) => request.status)).toEqual([200, 200, 200, 409]);
  });

  it.each([
    ['HEAD', '/api/private?secret=not-recorded'],
    ['POST', '/v1/undeclared?extra=true'],
    ['GET', '/v1/responses'],
    ['POST', '/v1/messages/count_tokens'],
  ])(
    'refuses unknown %s %s and latches failure instead of silently qualifying',
    async (method, route) => {
      const server = await start();
      const response = await fetch(server.baseUrl + route, { method });
      expect(response.status).toBe(404);
      if (method !== 'HEAD')
        expect(await response.json()).toMatchObject({ error: { code: 'MOCK_UNKNOWN_REQUEST' } });
      expect((await post(server, 'messages')).status).toBe(409);
      expect(server.snapshot()).toMatchObject({
        completedTurns: 0,
        failure: { code: 'MOCK_UNKNOWN_REQUEST' },
      });
      expect(JSON.stringify(server.snapshot())).not.toContain('secret=');
    }
  );

  it.each([
    ['yes', 'fixture-model', 'MOCK_REQUEST_SHAPE'],
    [true, 'different-model', 'MOCK_TURN_MISMATCH'],
  ])('rejects stream=%s / model=%s without consuming a turn', async (stream, model, code) => {
    const server = await start();
    const response = await post(server, 'messages', { ...body('messages'), stream, model });
    expect(await response.json()).toMatchObject({ error: { code } });
    expect(server.snapshot().completedTurns).toBe(0);
  });

  it('refuses the wrong protocol for the next scripted turn', async () => {
    const server = await start();
    expect(await (await post(server, 'responses')).json()).toMatchObject({
      error: { code: 'MOCK_TURN_MISMATCH' },
    });
    expect(server.snapshot().completedTurns).toBe(0);
  });

  it.each([
    [
      'authorization',
      'Bearer real-looking-token',
      'application/json',
      '{}',
      401,
      'MOCK_SYNTHETIC_AUTH_REQUIRED',
    ],
    ['x-api-key', MOCK_MODEL_TOKEN, 'text/plain', '{}', 415, 'MOCK_CONTENT_TYPE'],
    ['x-api-key', MOCK_MODEL_TOKEN, 'application/json', '{', 400, 'MOCK_INVALID_JSON'],
  ])(
    'refuses invalid auth, content type or JSON without recording secrets',
    async (header, token, contentType, payload, status, code) => {
      const server = await start();
      const response = await fetch(`${server.baseUrl}/v1/messages`, {
        method: 'POST',
        headers: { [header]: token, 'content-type': contentType },
        body: payload,
      });
      expect(response.status).toBe(status);
      expect(await response.json()).toMatchObject({ error: { code } });
      expect(JSON.stringify(server.snapshot())).not.toContain(token);
      expect(server.snapshot().completedTurns).toBe(0);
    }
  );

  it('accepts the synthetic Messages API key and Responses string input', async () => {
    const server = await start([turn('messages'), turn('responses')]);
    const response = await fetch(`${server.baseUrl}/v1/messages`, {
      method: 'POST',
      headers: { 'x-api-key': MOCK_MODEL_TOKEN, 'content-type': 'application/json' },
      body: JSON.stringify(body('messages')),
    });
    expect(response.status).toBe(200);
    await response.text();
    expect(
      (await post(server, 'responses', { ...body('responses'), input: 'fixture' })).status
    ).toBe(200);
  });

  it('bounds body bytes and request/census counts', async () => {
    const oversized = await start([turn('messages')], { maxBodyBytes: 8 });
    expect(await (await post(oversized, 'messages')).json()).toMatchObject({
      error: { code: 'MOCK_BODY_LIMIT' },
    });
    const bounded = await start([turn('messages')], { maxRequests: 1 });
    await fetch(`${bounded.baseUrl}/healthz`);
    for (let index = 0; index < 3; index++)
      expect((await post(bounded, 'messages')).status).toBe(429);
    expect(bounded.snapshot().census).toHaveLength(2);
    expect(bounded.snapshot().completedTurns).toBe(0);
  });

  it('times out an incomplete body with a named refusal and releases the socket', async () => {
    const server = await start([turn('messages')], { requestTimeoutMs: 50 });
    const socket = createConnection({
      host: '127.0.0.1',
      port: Number(new URL(server.baseUrl).port),
    });
    const chunks: Buffer[] = [];
    socket.on('data', (chunk) => chunks.push(Buffer.from(chunk)));
    await once(socket, 'connect');
    socket.write(
      `POST /v1/messages HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer ${MOCK_MODEL_TOKEN}\r\nContent-Type: application/json\r\nContent-Length: 1000\r\n\r\n{`
    );
    await once(socket, 'close');
    expect(Buffer.concat(chunks).toString()).toContain('MOCK_REQUEST_TIMEOUT');
    expect(server.snapshot().failure?.code).toBe('MOCK_REQUEST_TIMEOUT');
    expect(server.snapshot().completedTurns).toBe(0);
  });

  it('closes at its lifetime deadline and repeated close preserves the closed snapshot', async () => {
    const server = await start([turn('messages')], { timeoutMs: 30 });
    // This resolves from server.close's callback; rebinding the freed port races other listeners.
    await server.closed;
    const closedSnapshot = server.snapshot();
    expect(closedSnapshot).toMatchObject({
      closed: true,
      failure: { code: 'MOCK_SERVER_DEADLINE' },
    });
    await server.close();
    await server.close();
    expect(server.snapshot()).toEqual(closedSnapshot);
  });

  it('clones script/census state and refuses invalid script or bounds before opening a socket', async () => {
    const script = turn('messages');
    const server = await start([script]);
    script.model = 'mutated';
    expect((await post(server, 'messages')).status).toBe(200);
    server.snapshot().census.splice(0);
    expect(server.snapshot().census).toHaveLength(1);
    for (const bad of [0, 65, 1.5])
      await expect(
        startMockModelServer({ turns: [turn('messages')], limits: { maxRequests: bad } })
      ).rejects.toThrow('MOCK_CONFIG');
    await expect(startMockModelServer({ turns: [] })).rejects.toThrow('MOCK_CONFIG');
    await expect(
      startMockModelServer({ turns: [{ ...turn('responses'), cachedInputTokens: 14 }] })
    ).rejects.toThrow('MOCK_CONFIG');
    await expect(
      startMockModelServer({ turns: [{ ...turn('messages'), text: 'x'.repeat(8193) }] })
    ).rejects.toThrow('MOCK_CONFIG');
  });
});

it.each(['messages', 'responses'] as const)(
  'also supports the non-streaming %s form',
  async (protocol) => {
    const server = await start([turn(protocol)]);
    const response = await post(server, protocol, { ...body(protocol), stream: false });
    expect(response.status).toBe(200);
    expect(response.headers.get('content-type')).toBe('application/json');
    const result = await response.json();
    if (protocol === 'messages')
      expect(result).toMatchObject({
        type: 'message',
        content: [{ type: 'text', text: turn(protocol).text }],
        stop_reason: 'end_turn',
        usage: { input_tokens: 13, output_tokens: 7, cache_read_input_tokens: 3 },
      });
    else
      expect(result).toMatchObject({
        object: 'response',
        status: 'completed',
        output: [{ content: [{ type: 'output_text', text: turn(protocol).text }] }],
        usage: { input_tokens: 13, output_tokens: 7, total_tokens: 20 },
      });
    expect(server.snapshot().completedTurns).toBe(1);
  }
);

it('queues turns by protocol/model, preserving FIFO only within that key', async () => {
  const server = await start([
    turn('messages'),
    turn('responses'),
    { ...turn('messages'), text: 'second' },
  ]);
  expect((await post(server, 'responses')).status).toBe(200);
  expect(events(await (await post(server, 'messages')).text())[2]).toMatchObject({
    delta: { text: turn('messages').text },
  });
  expect(events(await (await post(server, 'messages')).text())[2]).toMatchObject({
    delta: { text: 'second' },
  });
  expect(server.snapshot()).toMatchObject({ completedTurns: 3 });
  expect(server.snapshot().failure).toBeUndefined();
});

it('counts declared side routes/models separately and never consumes scenario turns', async () => {
  const server = await startMockModelServer({
    turns: [turn('messages')],
    sideRequests: [
      { method: 'GET', path: '/v1/models', maxCalls: 1, response: { object: 'list', data: [] } },
      {
        method: 'POST',
        path: '/v1/messages/count_tokens',
        maxCalls: 1,
        response: { input_tokens: 13 },
      },
      {
        method: 'POST',
        path: '/v1/messages',
        maxCalls: 1,
        turn: { ...turn('messages'), model: 'title-model', text: 'Fixed title' },
      },
    ],
  });
  servers.push(server);
  expect(await (await fetch(`${server.baseUrl}/v1/models`)).json()).toEqual({
    object: 'list',
    data: [],
  });
  const headers = {
    authorization: `Bearer ${MOCK_MODEL_TOKEN}`,
    'content-type': 'application/json',
  };
  expect(
    await (
      await fetch(`${server.baseUrl}/v1/messages/count_tokens`, {
        method: 'POST',
        headers,
        body: '{}',
      })
    ).json()
  ).toEqual({ input_tokens: 13 });
  const title = await post(server, 'messages', { ...body('messages'), model: 'title-model' });
  expect(title.status).toBe(200);
  expect(events(await title.text())[2]).toMatchObject({ delta: { text: 'Fixed title' } });
  expect(server.snapshot()).toMatchObject({ sideRequestsSeen: 3, completedTurns: 0 });
  expect(server.snapshot().failure).toBeUndefined();
  expect(server.snapshot().census.at(-1)).toMatchObject({
    kind: 'side',
    model: 'title-model',
    stream: true,
  });
  expect((await post(server, 'messages')).status).toBe(200);
  expect(server.snapshot()).toMatchObject({ sideRequestsSeen: 3, completedTurns: 1 });
});

it('bounds declared side calls and rejects ambiguous side/turn ownership', async () => {
  const side = { method: 'GET' as const, path: '/v1/models', maxCalls: 1, response: { data: [] } };
  const server = await startMockModelServer({ turns: [turn('messages')], sideRequests: [side] });
  servers.push(server);
  await fetch(`${server.baseUrl}/v1/models`);
  const extra = await fetch(`${server.baseUrl}/v1/models`);
  expect(extra.status).toBe(429);
  expect(await extra.json()).toMatchObject({ error: { code: 'MOCK_SIDE_LIMIT' } });
  expect(server.snapshot()).toMatchObject({
    sideRequestsSeen: 1,
    completedTurns: 0,
    failure: { code: 'MOCK_SIDE_LIMIT' },
  });
  await expect(
    startMockModelServer({ turns: [turn('messages')], sideRequests: [side, side] })
  ).rejects.toThrow('MOCK_CONFIG');
  await expect(
    startMockModelServer({
      turns: [turn('messages')],
      sideRequests: [{ method: 'POST', path: '/v1/messages', maxCalls: 1, turn: turn('messages') }],
    })
  ).rejects.toThrow('MOCK_CONFIG');
  await expect(
    startMockModelServer({
      turns: [turn('messages')],
      sideRequests: [{ ...side, path: '/v1/messages' }],
    })
  ).rejects.toThrow('MOCK_CONFIG');
  await expect(
    startMockModelServer({ turns: [turn('messages')], sideRequests: [{ ...side, maxCalls: 17 }] })
  ).rejects.toThrow('MOCK_CONFIG');
  await expect(
    startMockModelServer({ turns: [{ ...turn('responses'), text: '\0'.repeat(8192) }] })
  ).rejects.toThrow('MOCK_CONFIG: response exceeds');
});

it('copies the script length and accepts omitted stream as non-streaming JSON', async () => {
  const script = [turn('responses')];
  const server = await start(script);
  script.push(turn('messages'));
  const response = await post(server, 'responses', { model: 'fixture-model', input: 'fixture' });
  expect(response.headers.get('content-type')).toBe('application/json');
  expect(await response.json()).toMatchObject({ status: 'completed' });
  expect(server.snapshot()).toMatchObject({ completedTurns: 1, plannedTurns: 1 });
});

it.each(['messages', 'responses'] as const)(
  'matches %s by pathname when SDK query flags are present',
  async (protocol) => {
    const server = await start([turn(protocol)]);
    const response = await fetch(`${server.baseUrl}/v1/${protocol}?beta=true`, {
      method: 'POST',
      headers: { authorization: `Bearer ${MOCK_MODEL_TOKEN}`, 'content-type': 'application/json' },
      body: JSON.stringify(body(protocol)),
      signal: AbortSignal.timeout(2000),
    });
    expect(response.status).toBe(200);
    await response.text();
    expect(server.snapshot()).toMatchObject({ completedTurns: 1 });
    expect(server.snapshot().failure).toBeUndefined();
    expect(server.snapshot().census[0].path).toBe(`/v1/${protocol}`);
  }
);

const sessionId = '12345678-1234-4234-8234-123456789abc';

it.each(['messages', 'responses'] as const)(
  'exposes only a canonical UUID from the selected pinned %s session field',
  async (protocol) => {
    const server = await startMockModelServer({
      turns: [turn(protocol)],
      sessionSource: protocol === 'messages' ? 'claude-code-2.1.285' : 'codex-0.160.0',
    });
    servers.push(server);
    const response = await fetch(`${server.baseUrl}/v1/${protocol}`, {
      method: 'POST',
      headers: {
        authorization: `Bearer ${MOCK_MODEL_TOKEN}`,
        'content-type': 'application/json',
        session_id: sessionId,
        'x-private-field': 'must-not-be-captured',
      },
      body: JSON.stringify({
        ...body(protocol),
        metadata: {
          user_id: JSON.stringify({ session_id: sessionId, account_id: 'must-not-be-captured' }),
        },
      }),
    });
    await response.text();
    const snapshot = server.snapshot();
    expect(snapshot.census[0].sessionId).toBe(sessionId);
    expect(JSON.stringify(snapshot)).not.toContain('must-not-be-captured');
    expect(Object.keys(snapshot.census[0]).sort()).toEqual(
      ['method', 'path', 'status', 'bodyBytes', 'model', 'stream', 'kind', 'sessionId'].sort()
    );
  }
);

it.each([undefined, 'not-a-uuid', sessionId.toUpperCase()])(
  'does not qualify a missing or non-canonical session field (%s)',
  async (value) => {
    const server = await startMockModelServer({
      turns: [turn('responses')],
      sessionSource: 'codex-0.160.0',
    });
    servers.push(server);
    const response = await fetch(`${server.baseUrl}/v1/responses`, {
      method: 'POST',
      headers: {
        authorization: `Bearer ${MOCK_MODEL_TOKEN}`,
        'content-type': 'application/json',
        ...(value === undefined ? {} : { session_id: value }),
      },
      body: JSON.stringify(body('responses')),
    });
    await response.text();
    expect(server.snapshot().census[0]).not.toHaveProperty('sessionId');
  }
);
