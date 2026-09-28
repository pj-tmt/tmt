import { useCallback, useContext, useState } from 'react';
import { useExtensionPanel } from '../extensions/use-extension-panel.js';
import { LocalRuntimeContext } from '../local/local-runtime.js';
import { NotebookView } from './notebook-view.js';

export function useNotebookPanel() {
  const runtime = useContext(LocalRuntimeContext);
  const [identityId, selectIdentity] = useState<string>();
  const { open: show, panel } = useExtensionPanel(
    'Notebook',
    (visible) =>
      visible &&
      identityId &&
      runtime && <NotebookView key={identityId} identityId={identityId} port={runtime.notebooks} />,
    'notebook-panel'
  );
  const open = useCallback(
    (id: string) => {
      selectIdentity(id);
      show();
    },
    [show]
  );
  return { open, panel };
}
