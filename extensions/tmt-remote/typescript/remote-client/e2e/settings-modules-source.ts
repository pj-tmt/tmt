/** Explicit injected records for Settings presentation tests, never the product source. */
import type { ShellAnswer } from '@tmt/browser-ui/shell';
import type { TmtSettings } from '../src/settings-source.js';

export function moduleFixture(mode: string, base: TmtSettings): TmtSettings {
  return {
    ...base,
    sample: mode === 'injected' || mode === 'late',
    modules:
      mode === 'refused'
        ? { rows: [], unavailable: 'Permission refused for module inventory.' }
        : {
            rows:
              mode === 'empty'
                ? []
                : [
                    {
                      name: 'colab',
                      installed: true,
                      version: '0.1.0-alpha.231',
                      channel: 'alpha',
                    },
                    {
                      name: 'remote',
                      installed: true,
                      version: '0.1.0-alpha.135',
                      channel: 'alpha',
                    },
                    { name: 'ops', installed: true, version: '0.1.0-alpha.158', channel: 'alpha' },
                    {
                      name: 'digest',
                      installed: true,
                      version: '0.1.0-alpha.12',
                      channel: 'alpha',
                    },
                    { name: 'office', installed: false },
                  ],
          },
  };
}

const module = (extension: string, label: string, order: number) => ({
  extension,
  rail: { label, icon: 'box', path: `/${extension}/`, order },
  menu: [],
  settings: [{ id: 'settings', label: 'Settings', path: `/${extension}/settings/` }],
});

/** What the composed shell would say about the installed modules. */
export const describedModules: ShellAnswer = {
  version: 1,
  modules: [
    module('colab', 'Colab', 1),
    module('remote', 'Remote', 2),
    module('ops', 'Ops', 3),
    module('digest', 'Digest', 4),
  ],
};
