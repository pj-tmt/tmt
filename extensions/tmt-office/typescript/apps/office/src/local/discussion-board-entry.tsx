import { useExtensionPanel } from '../extensions/use-extension-panel.js';
import { useCallback, useState } from 'react';
import type { ResourceBinding } from '../extensions/extension-contract.js';
import type { BoardCategory } from './board-contract.js';
import { LocalBoardPage } from './board-page.js';
import './discussion-board-entry.css';

/** One modal/draft owner shared by spatial and accessible entry points. */
export function useDiscussionBoard() {
  const [entryCategory, setEntryCategory] = useState<BoardCategory>({ kind: 'general' });
  const { open: show, panel } = useExtensionPanel(
    'Discussion board',
    <LocalBoardPage entryCategory={entryCategory} />,
    'discussion-board-panel'
  );
  const open = useCallback(
    (binding?: ResourceBinding) => {
      if (binding && binding.kind !== 'office-board') return;
      setEntryCategory(
        binding?.roomId ? { kind: 'room', roomId: binding.roomId } : { kind: 'general' }
      );
      show();
    },
    [show]
  );
  return { open, panel };
}
