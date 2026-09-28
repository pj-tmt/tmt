import { useCallback, useRef, useState } from 'react';
import { useExtensionPanel } from '../extensions/use-extension-panel.js';
import { WhiteboardEditor } from './editor.js';
import type { WhiteboardLeaveState } from './editor-state.js';

/** One resource session: closing retains its draft; changing resources is explicit. */
export function useWhiteboardPanel() {
  const [documentId, selectDocument] = useState<string>();
  const [requestedId, requestDocument] = useState<string>();
  const [leaveState, setLeaveState] = useState<WhiteboardLeaveState>('ready');
  const currentLeaveState = useRef<WhiteboardLeaveState>('ready');
  const updateLeaveState = useCallback((value: WhiteboardLeaveState) => {
    currentLeaveState.current = value;
    setLeaveState(value);
  }, []);
  const switchDocument = useCallback((id: string) => {
    selectDocument(id);
    requestDocument(undefined);
    currentLeaveState.current = 'ready';
    setLeaveState('ready');
  }, []);
  const { open: show, panel } = useExtensionPanel(
    'Whiteboard',
    <>
      {requestedId && (
        <section aria-label="Switch whiteboard confirmation">
          <h2>Open another whiteboard?</h2>
          <p>
            {leaveState === 'pending'
              ? 'Finish or resolve the current operation before switching. Submitted work may already have been applied.'
              : 'Switching discards the unsaved draft here. Saved whiteboard content is not deleted.'}
          </p>
          <button
            disabled={leaveState === 'pending'}
            onClick={() => {
              // Check live state too: a save can start after the switch request.
              if (currentLeaveState.current !== 'pending') switchDocument(requestedId);
            }}
          >
            Discard draft and switch
          </button>
          <button onClick={() => requestDocument(undefined)}>Keep this whiteboard</button>
        </section>
      )}
      {documentId && (
        <WhiteboardEditor
          key={documentId}
          documentId={documentId}
          onLeaveStateChange={updateLeaveState}
        />
      )}
    </>
  );
  const open = useCallback(
    (id: string) => {
      if (id === documentId) requestDocument(undefined);
      else if (documentId && currentLeaveState.current !== 'ready') requestDocument(id);
      else switchDocument(id);
      show();
    },
    [documentId, show, switchDocument]
  );
  return { open, panel };
}
