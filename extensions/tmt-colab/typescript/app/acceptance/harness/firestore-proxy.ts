import http from 'node:http';

interface Input {
  url: URL;
  method: string;
  headers: Record<string, string>;
  body: Buffer | null;
  signal: AbortSignal;
}
interface Reply {
  status: number;
  headers: Record<string, string>;
  body: Buffer;
}

/** Finite fixture HTTP responses use fresh connections, separate from pooled or long-poll traffic. */
export function finiteLoopbackResponse(input: Input): Promise<Reply> {
  if (input.url.protocol !== 'http:' || input.url.hostname !== '127.0.0.1')
    return Promise.reject(new Error('finite proxy requires a loopback HTTP server'));
  return new Promise((resolve, reject) => {
    const request = http.request(
      input.url,
      {
        method: input.method,
        headers: input.headers,
        agent: false,
        signal: AbortSignal.any([input.signal, AbortSignal.timeout(30_000)]),
      },
      (response) => {
        const parts: Buffer[] = [];
        response.on('data', (part: Buffer) => parts.push(part));
        response.once('error', reject);
        response.once('end', () => {
          const headers: Record<string, string> = {};
          for (const [name, value] of Object.entries(response.headers))
            if (value !== undefined)
              headers[name] = Array.isArray(value)
                ? value.join(name === 'set-cookie' ? '\n' : ', ')
                : value;
          resolve({ status: response.statusCode!, headers, body: Buffer.concat(parts) });
        });
      },
    );
    request.once('error', reject);
    request.end(input.body);
  });
}
