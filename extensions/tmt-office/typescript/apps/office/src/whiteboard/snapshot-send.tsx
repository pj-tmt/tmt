import { useEffect, useState } from 'react';
import type { DispatchPort } from '../local/dispatch-contract.js';
import type { ProfilePort } from '../profiles/profile-contract.js';
import type { RoomPort } from '../local/room-contract.js';
import { DispatchComposer } from '../local/dispatch-composer.js';
import { createSnapshotSendState } from './snapshot-send-state.js';
import type { SnapshotSendState } from './snapshot-send-state.js';

interface Props {
  snapshotId: string;
  dispatch: DispatchPort;
  profiles: ProfilePort;
  rooms: RoomPort;
  onDraftChange(dirty: boolean): void;
  onPendingChange?(pending: boolean): void;
}

export function WhiteboardSnapshotSend({ snapshotId, dispatch, onPendingChange, ...props }: Props) {
  const [state, setState] = useState<SnapshotSendState>();
  useEffect(() => {
    const owner = createSnapshotSendState(dispatch, snapshotId);
    setState(owner);
    return () => owner.dispose();
  }, [dispatch, snapshotId]);
  useEffect(() => {
    if (!state) return;
    const report = () => {
      const snapshot = state.getSnapshot();
      onPendingChange?.(
        snapshot.busy || (snapshot.attempted && !snapshot.receipt && !snapshot.rejected)
      );
    };
    report();
    const unsubscribe = state.subscribe(report);
    return () => {
      unsubscribe();
      onPendingChange?.(false);
    };
  }, [state, onPendingChange]);
  return state ? (
    <DispatchComposer
      {...props}
      state={state}
      copy={{
        label: 'Ask agents about this snapshot',
        heading: 'Ask agents',
        inputLabel: 'Question',
        placeholder: 'What should they help with?',
        audienceHint: 'Only these agents will receive this saved snapshot:',
      }}
    />
  ) : null;
}
