import { expect, it } from 'vite-plus/test';
import http from 'node:http';
import { finiteLoopbackResponse } from '../acceptance/harness/firestore-proxy.js';

async function server(handler: http.RequestListener) {
  const value = http.createServer(handler);
  await new Promise<void>((resolve, reject) => {
    value.once('error', reject);
    value.listen(0, '127.0.0.1', () => {
      value.off('error', reject);
      resolve();
    });
  });
  const { port } = value.address() as { port: number };
  return {
    url: new URL(`http://127.0.0.1:${port}/finite`),
    close: async () => {
      value.closeAllConnections();
      await new Promise<void>((resolve, reject) =>
        value.close((error) => (error ? reject(error) : resolve())),
      );
    },
  };
}
const call = (url: URL) => ({
  url,
  method: 'GET',
  headers: {},
  body: null,
  signal: new AbortController().signal,
});

it('uses one fresh connection per finite call when a server resets reused sockets', async () => {
  const sockets = new WeakSet<object>();
  let requests = 0;
  let connections = 0;
  const host = await server((request, response) => {
    requests++;
    if (sockets.has(request.socket)) {
      request.socket.destroy();
      return;
    }
    sockets.add(request.socket);
    connections++;
    response.end('original');
  });
  try {
    const first = await finiteLoopbackResponse(call(host.url));
    const second = await finiteLoopbackResponse(call(host.url));
    expect([first.body.toString(), second.body.toString()]).toEqual(['original', 'original']);
    expect(requests).toBe(2);
    expect(connections).toBe(2);
  } finally {
    await host.close();
  }
});

it('forwards the original method, headers, binary body and non-success response unchanged', async () => {
  const body = Buffer.from([0, 255, 1, 128]);
  const received: Buffer[] = [];
  const host = await server((request, response) => {
    expect(request.method).toBe('POST');
    expect(request.headers.authorization).toBe('Bearer fixture');
    expect(request.headers['content-type']).toBe('application/octet-stream');
    request.on('data', (part: Buffer) => received.push(part));
    request.on('end', () => {
      response.writeHead(409, {
        'content-type': 'application/octet-stream',
        'content-length': String(body.length),
        'x-proof': 'unchanged',
        date: 'Thu, 01 Jan 1970 00:00:00 GMT',
        connection: 'close',
      });
      response.end(body);
    });
  });
  try {
    const result = await finiteLoopbackResponse({
      ...call(host.url),
      method: 'POST',
      headers: { authorization: 'Bearer fixture', 'content-type': 'application/octet-stream' },
      body,
    });
    expect(Buffer.concat(received)).toEqual(body);
    expect(result).toEqual({
      status: 409,
      headers: {
        'content-type': 'application/octet-stream',
        'content-length': '4',
        'x-proof': 'unchanged',
        date: 'Thu, 01 Jan 1970 00:00:00 GMT',
        connection: 'close',
      },
      body,
    });
  } finally {
    await host.close();
  }
});

it('rejects an upstream reset after one request instead of retrying or inventing a response', async () => {
  let requests = 0;
  const host = await server((request) => {
    requests++;
    request.socket.destroy();
  });
  try {
    await expect(finiteLoopbackResponse(call(host.url))).rejects.toMatchObject({
      code: 'ECONNRESET',
    });
    expect(requests).toBe(1);
  } finally {
    await host.close();
  }
});

it('refuses a non-loopback target without making a request', async () => {
  await expect(
    finiteLoopbackResponse(call(new URL('https://firestore.googleapis.com/'))),
  ).rejects.toThrow('loopback HTTP server');
});

it('aborts and closes a pending upstream request when the proof is disposed', async () => {
  let started!: () => void;
  let closed!: () => void;
  const arrived = new Promise<void>((resolve) => {
    started = resolve;
  });
  const finished = new Promise<void>((resolve) => {
    closed = resolve;
  });
  const host = await server((_request, response) => {
    response.once('close', closed);
    started();
  });
  const controller = new AbortController();
  try {
    const pending = finiteLoopbackResponse({ ...call(host.url), signal: controller.signal });
    const rejected = expect(pending).rejects.toMatchObject({ name: 'AbortError' });
    await arrived;
    controller.abort();
    await rejected;
    await finished;
  } finally {
    controller.abort();
    await host.close();
  }
});
