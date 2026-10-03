import type { PageTransport } from './transport.js';
import { discover, mountUrl } from './bootstrap.js';
import { register, remoteSdk } from './registration.js';
import { Live } from './live.js';
import { verifyRegistration } from './registration.js';
import { ManagementClient, project } from './management.js';
import { text } from './strings.js';

export async function mountedTransport(): Promise<{ space: string; transport: PageTransport }> {
  const mount = mountUrl(),
    registration = await register(mount, await remoteSdk()),
    bootstrap = await discover(mount, (space, owner) =>
      verifyRegistration(registration, space, owner),
    );
  const management = new ManagementClient(mount, registration);
  return {
    space: bootstrap.space,
    transport: {
      management,
      async spaceHome() {
        const { boot, log } = await management.snapshot();
        return {
          title: text.product,
          pages: boot.pages
            .filter((page) => !project(page, log).page.archived)
            .map((page) => {
              const policy = project(page, log).page;
              return {
                id: page.pageId,
                title: page.pageId,
                sharing: policy.sharing,
              };
            }),
        };
      },
      async page(id, signal) {
        const current = await discover(
          mount,
          (space, owner) => verifyRegistration(registration, space, owner),
          signal,
        );
        const page = current.pages.find((page) => page.pageId === id && !page.archived);
        if (!page) throw new Error('Page unavailable');
        const live = new Live(mount, current, registration, page, signal);
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
