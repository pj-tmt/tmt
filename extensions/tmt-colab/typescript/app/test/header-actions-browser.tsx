import { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { BrowserHeader, BrowserIconAction } from '@tmt/browser-ui/react';
import { Settings } from 'lucide-react';

function Actions() {
  const [count, setCount] = useState(0);
  return (
    <>
      <BrowserHeader
        productLabel="Colab"
        title="Action focus"
        actions={
          <>
            <BrowserIconAction
              type="button"
              label="Header action"
              variant="text"
              icon={<Settings />}
              onActivate={() => setCount((value) => value + 1)}
            />
            <button type="button">Next focus target</button>
          </>
        }
      />
      <output data-testid="header-count">{count}</output>
    </>
  );
}

export function mount() {
  document.getElementById('root')?.setAttribute('hidden', '');
  const host = document.createElement('main');
  document.body.append(host);
  createRoot(host).render(<Actions />);
}
