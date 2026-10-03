import type { PageTransport } from './transport.js';
import { discover, mountUrl } from './bootstrap.js';
import { register, remoteSdk } from './registration.js';
import { Live } from './live.js';
import { verifyRegistration } from './registration.js';
import { text } from './strings.js';

export async function mountedTransport(): Promise<{ space: string; transport: PageTransport }> {
  const mount = mountUrl(),
    registration = await register(mount, await remoteSdk()),
    bootstrap = await discover(mount, (space, owner) =>
      verifyRegistration(registration, space, owner),
    );
  return {
    space: bootstrap.space,
    transport: {
      async spaceHome() {
        return {
          title: text.product,
          pages: bootstrap.pages
            .filter((page) => !page.archived)
            .map((page) => ({
              id: page.pageId,
              title: page.pageId,
              sharing: page.sharing,
            })),
        };
      },
      async page(id, signal) {
        const page = bootstrap.pages.find((page) => page.pageId === id && !page.archived);
        if (!page) throw new Error('Page unavailable');
        const live = new Live(mount, bootstrap, registration, page, signal);
        try {
          return await live.snapshot();
        } catch (error) {
          live.close();
          throw error;
        }
      },
    },
  };
}
