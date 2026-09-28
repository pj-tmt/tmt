import { expect, test } from '@playwright/test';
import deployments from '../../../../contracts/deployment-examples.json' with { type: 'json' };

test('built emulator serves only the public descriptor; preview and unconfigured cloud do not', async ({
  request,
}) => {
  const path = '/.well-known/tmt-office.json';
  const response = await request.get(`http://127.0.0.1:4173${path}`, { maxRedirects: 0 });
  expect(response.status()).toBe(200);
  expect(response.headers()['content-type']).toContain('application/json');
  expect(await response.json()).toEqual(deployments.valid[1].descriptor);
  for (const port of [4174, 4175]) {
    const unavailable = await request.get(`http://127.0.0.1:${port}${path}`, { maxRedirects: 0 });
    // Static SPA hosting may rewrite missing assets to index.html. Such a
    // response is not a descriptor and native discovery must reject its MIME.
    expect(unavailable.headers()['content-type']).not.toContain('application/json');
    expect(await unavailable.text()).not.toContain('"pairingUrl"');
  }
});
