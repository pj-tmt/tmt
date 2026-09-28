import { useContext, useEffect, useState } from 'react';
import { LocalRuntimeContext } from './local-runtime.js';
import { DispatchComposer } from './dispatch-composer.js';
import { createDispatchComposerState } from './dispatch-composer-state.js';
import type { DispatchComposerState } from './dispatch-composer-state.js';

/** The spatial broadcaster composes no-reply messages over the shared host capability. */
export function Broadcaster() {
  const runtime = useContext(LocalRuntimeContext);
  const [state, setState] = useState<DispatchComposerState>();
  useEffect(() => {
    if (!runtime) return;
    const owner = createDispatchComposerState(runtime.dispatch, {
      kind: 'announcement',
      message: (text) => text,
    });
    setState(owner);
    return () => owner.dispose();
  }, [runtime]);
  if (!runtime) return <p role="alert">Start a local Office session to use the broadcaster.</p>;
  return (
    <div className="office-broadcaster">
      <header>
        <span className="broadcaster-channel">OFFICE / BROADCAST</span>
        <h2>A message for your team.</h2>
        <p>Choose who hears it. Send a notification, without requesting replies.</p>
      </header>
      {state && (
        <DispatchComposer
          state={state}
          profiles={runtime.profiles}
          rooms={runtime.rooms}
          copy={{
            label: 'Compose an announcement',
            heading: 'Compose announcement',
            inputLabel: 'Message',
            placeholder: 'What would you like to share?',
            audienceHint: 'Only these agents will receive this announcement:',
          }}
        />
      )}
    </div>
  );
}
