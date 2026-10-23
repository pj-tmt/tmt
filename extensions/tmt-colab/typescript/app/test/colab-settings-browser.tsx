/** Isolated settings source: no native settings, transport or writes. */
import 'virtual:tokens.css';
import '@tmt/browser-ui/static.css';
import '../src/style.css';
import { createRoot, type Root } from 'react-dom/client';
import { ColabSettings } from '../src/colab-settings.js';
import type { ColabSettingsView } from '../src/colab-settings-source.js';

let root: Root | undefined;
export function mount(view: ColabSettingsView) {
  document.getElementById('root')!.style.display = 'none';
  let host = document.getElementById('settings-fixture');
  if (!host) {
    host = document.createElement('div');
    host.id = 'settings-fixture';
    document.body.append(host);
  }
  root?.unmount();
  root = createRoot(host);
  root.render(
    <ColabSettings
      source={{
        async read() {
          return view;
        },
      }}
    />,
  );
}
