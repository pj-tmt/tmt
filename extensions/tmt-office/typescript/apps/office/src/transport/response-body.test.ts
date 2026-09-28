import { expect, it, vi } from 'vitest';
import { readBoundedBody } from './response-body.js';

it('joins exact-limit chunks without trusting content length', async () => {
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      controller.enqueue(new Uint8Array([1, 2]));
      controller.enqueue(new Uint8Array([3, 4]));
      controller.close();
    },
  });
  expect(
    await readBoundedBody(new Response(body, { headers: { 'Content-Length': '1' } }), 4)
  ).toEqual(new Uint8Array([1, 2, 3, 4]));
  expect(body.locked).toBe(false);
});

it('cancels oversize bodies and releases the reader on success or read failure', async () => {
  const cancel = vi.fn();
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      controller.enqueue(new Uint8Array(5));
    },
    cancel,
  });
  await expect(readBoundedBody(new Response(body), 4)).rejects.toThrow('exceeds');
  expect(cancel).toHaveBeenCalledOnce();
  expect(body.locked).toBe(false);
  const broken = new ReadableStream({
    start(controller) {
      controller.error(new Error('broken body'));
    },
  });
  await expect(readBoundedBody(new Response(broken), 4)).rejects.toThrow('broken body');
  expect(broken.locked).toBe(false);
});
