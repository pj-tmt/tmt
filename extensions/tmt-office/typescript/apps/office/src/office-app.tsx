import { RouterProvider } from '@tanstack/react-router';
import { createStore, Provider } from 'jotai';
import { useState } from 'react';
import type { ReactElement } from 'react';
import type { createOfficeRouter } from './router.js';
import { OfficeSessionContext, OfficeModeContext } from './auth/session-view.js';
import type { OfficeSession } from './auth/session.js';
import { WorldContext } from './worlds/world-view.js';
import type { WorldState } from './worlds/world-state.js';
import { BlockContext } from './blocks/block-view.js';
import type { BlockPort } from './blocks/block-contract.js';
import { PairingContext } from './pairing/pairing-view.js';
import type { PairingPort } from './pairing/pairing-contract.js';
import { SpaceContext } from './spaces/space-view.js';
import type { SpacePort } from './spaces/space-contract.js';
import { LocalRuntimeContext } from './local/local-runtime.js';
import type { LocalRuntime } from './local/local-runtime.js';

export function OfficeApp({
  router,
  session,
  worlds,
  blocks,
  pairing,
  spaces,
  local,
  mode = 'emulator',
}: {
  router: ReturnType<typeof createOfficeRouter>;
  session?: OfficeSession;
  worlds?: WorldState;
  blocks?: BlockPort;
  pairing?: PairingPort;
  spaces?: SpacePort;
  local?: LocalRuntime;
  mode?: string;
}): ReactElement {
  // A mounted app owns its UI state; tests and future embedded views cannot leak it.
  const [store] = useState(() => createStore());
  return (
    <Provider store={store}>
      <OfficeSessionContext value={session}>
        <OfficeModeContext value={mode}>
          <WorldContext value={worlds}>
            <BlockContext value={blocks}>
              <PairingContext value={pairing}>
                <SpaceContext value={spaces}>
                  <LocalRuntimeContext value={local}>
                    <RouterProvider router={router} />
                  </LocalRuntimeContext>
                </SpaceContext>
              </PairingContext>
            </BlockContext>
          </WorldContext>
        </OfficeModeContext>
      </OfficeSessionContext>
    </Provider>
  );
}
