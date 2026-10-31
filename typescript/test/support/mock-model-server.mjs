import { createHash } from 'node:crypto';
import { createServer } from 'node:http';

export const MOCK_MODEL_TOKEN = 'tmt-synthetic-model-token';
const ceilings = {
  maxRequests: 64,
  maxBodyBytes: 1048576,
  timeoutMs: 120000,
  requestTimeoutMs: 5000,
};

function boundedInteger(value, maximum, name) {
  if (!Number.isSafeInteger(value) || value < 1 || value > maximum)
    throw new Error(`MOCK_CONFIG: ${name} must be an integer in 1..${maximum}`);
  return value;
}

function validModel(model) {
  return typeof model === 'string' && /^[a-zA-Z0-9_.:-]{1,128}$/.test(model);
}

function validInputText(text) {
  return typeof text === 'string' && text.trim().length > 0 && Buffer.byteLength(text) <= 8192;
}

/** Approved synthetic-input diagnostics only; no request/header object reaches this seam. */
export function describeMockInputMismatch(parsed, scripted) {
  if ((parsed !== undefined && typeof parsed !== 'string') || typeof scripted !== 'string')
    throw new TypeError('MOCK_DIAGNOSTIC: parsed text and scripted text only');
  const actual = parsed === undefined ? undefined : Buffer.from(parsed);
  const expected = Buffer.from(scripted);
  const summary = (bytes) => ({
    bytes: bytes.length,
    sha256: createHash('sha256').update(bytes).digest('hex'),
  });
  let firstDifferingByte = null;
  if (actual) {
    let i = 0;
    while (i < actual.length && i < expected.length && actual[i] === expected[i]) i++;
    if (i < actual.length || i < expected.length) firstDifferingByte = i;
  }
  const excerpt = (text) => {
    if (text === undefined || (actual && firstDifferingByte === null)) return '';
    // Replace before slicing, preserving byte offsets even if a window cuts through the token.
    const bytes = Buffer.from(
      text.split(MOCK_MODEL_TOKEN).join('*'.repeat(MOCK_MODEL_TOKEN.length))
    );
    let start = Math.max(0, Math.min(bytes.length - 256, (firstDifferingByte ?? 0) - 128));
    let end = Math.min(bytes.length, start + 256);
    while (start < end && (bytes[start] & 0xc0) === 0x80) start++;
    while (end > start && (bytes[end] & 0xc0) === 0x80) end--;
    return bytes.subarray(start, end).toString('utf8');
  };
  return {
    parsed: actual
      ? { state: 'text', ...summary(actual) }
      : { state: 'missing_or_unsupported', bytes: null, sha256: null },
    scripted: summary(expected),
    firstDifferingByte,
    excerpts: { parsed: excerpt(parsed), scripted: excerpt(scripted) },
  };
}

function lastUserText(input) {
  if (typeof input === 'string') return input.trim() || undefined;
  const content = input.findLast((item) => item?.role === 'user')?.content;
  if (typeof content === 'string') return content.trim() || undefined;
  if (
    !Array.isArray(content) ||
    !content.length ||
    content.some(
      (part) => !['text', 'input_text'].includes(part?.type) || typeof part.text !== 'string'
    )
  )
    return undefined;
  return (
    content
      .map((part) => part.text)
      .join('\n')
      .trim() || undefined
  );
}

function compileTurn(turn, index) {
  if (!turn || !['messages', 'responses'].includes(turn.protocol))
    throw new Error('MOCK_CONFIG: unsupported turn protocol');
  if (!validModel(turn.model)) throw new Error('MOCK_CONFIG: invalid model');
  if (turn.inputText !== undefined && !validInputText(turn.inputText))
    throw new Error('MOCK_CONFIG: inputText must be nonempty and at most 8192 UTF-8 bytes');
  if (typeof turn.text !== 'string' || Buffer.byteLength(turn.text) > 8192)
    throw new Error('MOCK_CONFIG: text must be at most 8192 UTF-8 bytes');
  for (const name of ['inputTokens', 'outputTokens', 'cachedInputTokens']) {
    const value = turn[name] ?? (name === 'cachedInputTokens' ? 0 : undefined);
    if (!Number.isSafeInteger(value) || value < 0 || value > 1000000)
      throw new Error(`MOCK_CONFIG: invalid ${name}`);
  }
  const { protocol, model, text, inputTokens, outputTokens } = turn;
  const cachedInputTokens = turn.cachedInputTokens ?? 0;
  if (protocol === 'responses' && cachedInputTokens > inputTokens)
    throw new Error('MOCK_CONFIG: cached input exceeds Responses input');
  const id = `msg_mock_${index + 1}`;
  let events;
  let result;
  if (protocol === 'messages') {
    events = [
      {
        type: 'message_start',
        message: {
          id,
          type: 'message',
          role: 'assistant',
          model,
          content: [],
          stop_reason: null,
          stop_sequence: null,
          usage: {
            input_tokens: inputTokens,
            output_tokens: 0,
            cache_read_input_tokens: cachedInputTokens,
            cache_creation_input_tokens: 0,
          },
        },
      },
      { type: 'content_block_start', index: 0, content_block: { type: 'text', text: '' } },
      { type: 'content_block_delta', index: 0, delta: { type: 'text_delta', text } },
      { type: 'content_block_stop', index: 0 },
      {
        type: 'message_delta',
        delta: { stop_reason: 'end_turn', stop_sequence: null },
        usage: { output_tokens: outputTokens },
      },
      { type: 'message_stop' },
    ];
    result = {
      ...events[0].message,
      content: [{ type: 'text', text }],
      stop_reason: 'end_turn',
      usage: { ...events[0].message.usage, output_tokens: outputTokens },
    };
  } else {
    const part = { type: 'output_text', text, annotations: [], logprobs: [] };
    const item = { id, type: 'message', role: 'assistant', status: 'completed', content: [part] };
    const response = {
      id: `resp_mock_${index + 1}`,
      object: 'response',
      created_at: 0,
      model,
      status: 'in_progress',
      error: null,
      incomplete_details: null,
      output: [],
      usage: null,
    };
    const position = { item_id: id, output_index: 0, content_index: 0 };
    events = [
      { type: 'response.created', response },
      { type: 'response.in_progress', response },
      {
        type: 'response.output_item.added',
        output_index: 0,
        item: { ...item, status: 'in_progress', content: [] },
      },
      { type: 'response.content_part.added', ...position, part: { ...part, text: '' } },
      { type: 'response.output_text.delta', ...position, delta: text, logprobs: [] },
      { type: 'response.output_text.done', ...position, text, logprobs: [] },
      { type: 'response.content_part.done', ...position, part },
      { type: 'response.output_item.done', output_index: 0, item },
      {
        type: 'response.completed',
        response: {
          ...response,
          status: 'completed',
          output: [item],
          usage: {
            input_tokens: inputTokens,
            output_tokens: outputTokens,
            total_tokens: inputTokens + outputTokens,
            input_tokens_details: { cached_tokens: cachedInputTokens },
            output_tokens_details: { reasoning_tokens: 0 },
          },
        },
      },
    ].map((event, sequence_number) => ({ ...event, sequence_number }));
    result = events.at(-1).response;
  }
  const wire = events
    .map((event) => `event: ${event.type}\ndata: ${JSON.stringify(event)}\n\n`)
    .join('');
  if (Buffer.byteLength(wire) > 65536) throw new Error('MOCK_CONFIG: response exceeds 65536 bytes');
  return { protocol, model, inputText: turn.inputText?.trim(), wire, json: JSON.stringify(result) };
}

function compileSideRequest(side, index) {
  if (
    !side ||
    !['GET', 'HEAD', 'POST'].includes(side.method) ||
    typeof side.path !== 'string' ||
    !/^\/[a-zA-Z0-9/_-]{1,127}$/.test(side.path) ||
    side.path === '/healthz'
  )
    throw new Error('MOCK_CONFIG: invalid side-request route');
  const maxCalls = boundedInteger(side.maxCalls, 16, 'side maxCalls');
  if (side.turn) {
    const turn = compileTurn(side.turn, 1000 + index);
    if (side.method !== 'POST' || side.path !== `/v1/${turn.protocol}` || 'response' in side)
      throw new Error('MOCK_CONFIG: side text must match its protocol POST route');
    if (
      side.unlessLastUserText !== undefined &&
      (!validInputText(side.unlessLastUserText) || maxCalls > 2)
    )
      throw new Error('MOCK_CONFIG: discriminated side text needs an input and at most two calls');
    return {
      ...side,
      unlessLastUserText: side.unlessLastUserText?.trim(),
      turn,
      model: turn.model,
      maxCalls,
      calls: 0,
    };
  }
  if (side.unlessLastUserText !== undefined)
    throw new Error('MOCK_CONFIG: input discrimination requires a text-turn side reply');
  if (['/v1/messages', '/v1/responses'].includes(side.path))
    throw new Error('MOCK_CONFIG: model routes need a text-turn reply');
  let json;
  try {
    json = JSON.stringify(side.response);
  } catch {
    throw new Error('MOCK_CONFIG: side response must be JSON');
  }
  if (json === undefined || Buffer.byteLength(json) > 65536)
    throw new Error('MOCK_CONFIG: side response exceeds 65536 bytes or is not JSON');
  if (side.model !== undefined && (side.method !== 'POST' || typeof side.model !== 'string'))
    throw new Error('MOCK_CONFIG: a side model requires a POST route');
  return { method: side.method, path: side.path, model: side.model, json, maxCalls, calls: 0 };
}

/** Scripted transport only: never launches a provider, reads its state, or forwards traffic. */
export async function startMockModelServer({
  turns,
  sideRequests = [],
  limits = {},
  sessionSource,
  onInputMismatch,
}) {
  if (onInputMismatch !== undefined && typeof onInputMismatch !== 'function')
    throw new TypeError('MOCK_CONFIG: onInputMismatch must be a diagnostic callback');
  if (
    sessionSource !== undefined &&
    !['claude-code-2.1.285', 'codex-0.160.0'].includes(sessionSource)
  )
    throw new Error('MOCK_CONFIG: session source must name an exact supported client pin');
  if (!Array.isArray(turns) || turns.length < 1 || turns.length > 16)
    throw new Error('MOCK_CONFIG: supply 1..16 scripted text turns');
  const key = (protocol, model) => JSON.stringify([protocol, model]);
  const plannedTurns = turns.length;
  const queues = new Map();
  turns.map(compileTurn).forEach((turn) => {
    const id = key(turn.protocol, turn.model);
    if (!queues.has(id)) queues.set(id, []);
    queues.get(id).push(turn);
  });
  if (!Array.isArray(sideRequests) || sideRequests.length > 16)
    throw new Error('MOCK_CONFIG: at most 16 declared side requests');
  const sides = sideRequests.map(compileSideRequest);
  const routes = [];
  for (const side of sides) {
    const ownedTurns = side.turn && queues.get(key(side.turn.protocol, side.model));
    const discriminated = side.unlessLastUserText !== undefined;
    if (
      discriminated &&
      (!ownedTurns || ownedTurns.some((turn) => turn.inputText !== side.unlessLastUserText))
    )
      throw new Error('MOCK_CONFIG: discriminated side request needs matching scripted input');
    if (
      routes.some(
        (row) =>
          row.method === side.method &&
          row.path === side.path &&
          (row.model === undefined || side.model === undefined || row.model === side.model)
      ) ||
      (ownedTurns && !discriminated)
    )
      throw new Error('MOCK_CONFIG: duplicate or overlapping side request');
    routes.push(side);
  }
  for (const name of Object.keys(limits))
    if (!Object.hasOwn(ceilings, name)) throw new Error(`MOCK_CONFIG: unknown limit ${name}`);
  const bounds = Object.fromEntries(
    Object.entries(ceilings).map(([name, ceiling]) => [
      name,
      boundedInteger(limits[name] ?? (name === 'maxRequests' ? 16 : ceiling), ceiling, name),
    ])
  );
  const census = [];
  let requestsSeen = 0;
  let completedTurns = 0;
  let sideRequestsSeen = 0;
  let failure;
  let stopped = false;
  let closing = false;
  let timer;
  let resolveClosed;
  const closed = new Promise((resolve) => {
    resolveClosed = resolve;
  });
  const fail = (code, message) => {
    failure ??= { code, message };
  };
  const server = createServer((request, response) => {
    requestsSeen++;
    const pathname = (request.url ?? '').split('?')[0];
    const entry = {
      method: request.method ?? '',
      path: pathname.slice(0, 128),
      status: 0,
      bodyBytes: 0,
    };
    if (census.length < bounds.maxRequests + 1) census.push(entry);
    const refuse = (status, code, message) => {
      if (response.writableEnded || response.destroyed) return;
      fail(code, message);
      entry.status = status;
      entry.code = code;
      response.writeHead(status, { 'content-type': 'application/json', connection: 'close' });
      response.end(JSON.stringify({ error: { type: 'mock_model_error', code, message } }), () => {
        if (!request.complete) request.destroy();
      });
      request.resume();
    };
    const send = (json, wire) => {
      entry.status = 200;
      response.writeHead(200, {
        'content-type': wire ? 'text/event-stream; charset=utf-8' : 'application/json',
        'cache-control': 'no-cache',
        connection: 'close',
      });
      response.end(wire ?? json);
    };
    const sendSide = (side, stream) => {
      if (side.calls >= side.maxCalls)
        return refuse(429, 'MOCK_SIDE_LIMIT', 'Declared side-request budget exhausted');
      side.calls++;
      sideRequestsSeen++;
      entry.kind = 'side';
      send(side.turn?.json ?? side.json, stream ? side.turn.wire : undefined);
    };
    if (requestsSeen > bounds.maxRequests)
      return refuse(429, 'MOCK_REQUEST_LIMIT', 'Request budget exhausted');
    if (failure) return refuse(409, 'MOCK_FAILED', 'An earlier request failed; inspect the census');
    if (request.method === 'GET' && pathname === '/healthz') {
      entry.kind = 'health';
      send('{"ready":true}');
      return;
    }
    const protocol =
      pathname === '/v1/messages'
        ? 'messages'
        : pathname === '/v1/responses'
          ? 'responses'
          : undefined;
    const sideRoutes = sides.filter(
      (side) => side.method === request.method && side.path === pathname
    );
    if (request.method !== 'POST') {
      if (sideRoutes.length) return sendSide(sideRoutes[0], false);
      return refuse(404, 'MOCK_UNKNOWN_REQUEST', 'Request route is not declared');
    }
    if (!protocol && !sideRoutes.length)
      return refuse(404, 'MOCK_UNKNOWN_REQUEST', 'Request route is not declared');
    if (
      request.headers.authorization !== `Bearer ${MOCK_MODEL_TOKEN}` &&
      !(pathname.startsWith('/v1/messages') && request.headers['x-api-key'] === MOCK_MODEL_TOKEN)
    )
      return refuse(401, 'MOCK_SYNTHETIC_AUTH_REQUIRED', 'Use only the fixture synthetic token');
    if (!/^application\/json(?:;|$)/i.test(request.headers['content-type'] ?? ''))
      return refuse(415, 'MOCK_CONTENT_TYPE', 'Expected application/json');
    const chunks = [];
    const bodyTimer = setTimeout(() => {
      refuse(408, 'MOCK_REQUEST_TIMEOUT', 'Request body deadline exceeded');
    }, bounds.requestTimeoutMs);
    request.once('close', () => clearTimeout(bodyTimer));
    request.on('error', () => fail('MOCK_REQUEST_ABORTED', 'Request failed before completion'));
    request.on('data', (chunk) => {
      entry.bodyBytes += chunk.length;
      if (entry.bodyBytes > bounds.maxBodyBytes)
        refuse(413, 'MOCK_BODY_LIMIT', 'Request body exceeds the byte limit');
      else if (!response.writableEnded) chunks.push(chunk);
    });
    request.on('end', () => {
      clearTimeout(bodyTimer);
      if (response.writableEnded) return;
      let body;
      try {
        body = JSON.parse(Buffer.concat(chunks).toString('utf8'));
      } catch {
        return refuse(400, 'MOCK_INVALID_JSON', 'Request body is not JSON');
      }
      if (validModel(body?.model)) entry.model = body.model;
      if (typeof body?.stream === 'boolean') entry.stream = body.stream;
      // Candidate wire locations stay unqualified until a hosted request matches public CLI JSON.
      // Never retain the containing metadata, header, or a non-canonical identifier.
      let sessionId;
      if (sessionSource === 'codex-0.160.0' && protocol === 'responses')
        sessionId = request.headers.session_id;
      if (sessionSource === 'claude-code-2.1.285' && protocol === 'messages') {
        try {
          sessionId = JSON.parse(body?.metadata?.user_id).session_id;
        } catch {
          // Missing or moved fields remain absent; the advisory assertion refuses qualification.
        }
      }
      if (
        typeof sessionId === 'string' &&
        /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(sessionId)
      )
        entry.sessionId = sessionId;
      // Recheck after asynchronous reads: a refusal on another connection is fatal to the fixture.
      if (failure)
        return refuse(409, 'MOCK_FAILED', 'An earlier request failed; inspect the census');
      const side = sideRoutes.find((row) => row.model === undefined || row.model === body?.model);
      if (side && !side.turn) return sendSide(side, false);
      const input = protocol === 'messages' ? body?.messages : body?.input;
      if (
        !protocol ||
        !body ||
        (body.stream !== undefined && typeof body.stream !== 'boolean') ||
        !(Array.isArray(input)
          ? input.length > 0
          : protocol === 'responses' && typeof input === 'string' && input.length > 0)
      )
        return refuse(400, 'MOCK_REQUEST_SHAPE', 'Expected a text-turn request with input');
      const userText = lastUserText(input);
      if (
        side &&
        (side.unlessLastUserText === undefined ||
          (userText !== undefined &&
            userText !== side.unlessLastUserText &&
            (body.tools === undefined || (Array.isArray(body.tools) && body.tools.length === 0))))
      )
        return sendSide(side, body.stream === true);
      const queue = queues.get(key(protocol, body.model));
      if (!queue) return refuse(409, 'MOCK_TURN_MISMATCH', 'No script for this protocol/model');
      if (!queue.length)
        return refuse(
          409,
          'MOCK_SCRIPT_EXHAUSTED',
          'No scripted turn remains for this protocol/model'
        );
      if (queue[0].inputText !== undefined && userText !== queue[0].inputText) {
        onInputMismatch?.(describeMockInputMismatch(userText, queue[0].inputText));
        return refuse(409, 'MOCK_TURN_INPUT', 'Last user text does not match the scripted input');
      }
      const turn = queue.shift();
      completedTurns++;
      entry.kind = 'turn';
      send(turn.json, body.stream === true ? turn.wire : undefined);
    });
  });
  server.maxConnections = 4;
  server.headersTimeout = bounds.requestTimeoutMs;
  server.requestTimeout = bounds.requestTimeoutMs;
  const close = () => {
    if (!closing) {
      closing = true;
      clearTimeout(timer);
      server.close(() => {
        stopped = true;
        resolveClosed();
      });
      server.closeAllConnections();
    }
    return closed;
  };
  server.on('drop', () => fail('MOCK_CONNECTION_LIMIT', 'Connection budget exhausted'));
  server.on('clientError', (_error, socket) => {
    fail('MOCK_HTTP_PARSE', 'Invalid HTTP framing');
    socket.destroy();
  });
  await new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolve);
  });
  timer = setTimeout(() => {
    fail('MOCK_SERVER_DEADLINE', 'Mock server lifetime exhausted');
    void close();
  }, bounds.timeoutMs);
  timer.unref();
  return {
    baseUrl: `http://127.0.0.1:${server.address().port}`,
    close,
    closed,
    snapshot: () => ({
      requestsSeen,
      sideRequestsSeen,
      completedTurns,
      plannedTurns,
      closed: stopped,
      ...(failure ? { failure: { ...failure } } : {}),
      census: census.map((row) => ({ ...row })),
    }),
  };
}
