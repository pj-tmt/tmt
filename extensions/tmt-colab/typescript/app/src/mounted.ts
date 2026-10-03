import type { PageTransport } from './transport.js';
import { discover, mountUrl } from './bootstrap.js';
import { register, remoteSdk } from './registration.js';
import { Admission } from './admission.js';
import { bootstrapPage } from './catchup.js';
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
        const admission = new Admission(
          bootstrap.space,
          id,
          page.epoch,
          bootstrap.owner,
          registration,
        );
        try {
          await bootstrapPage(mount, admission, page.sharing, signal);
          if (!admission.root) throw new Error(text.noWraps);
          throw new Error(text.livePending);
        } finally {
          admission.root = null;
        }
      },
    },
  };
}
