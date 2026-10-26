/** Test-only entry: the Ops agents view on a bare page, fed boards by the spec. */
import { agentsView } from '../src/ops-agents.js';
import type { OpsBoard } from '../src/ops-source.js';

declare global {
  interface Window {
    opsUpdate(board: OpsBoard): void;
  }
}

const view = agentsView();
document.body.append(view.node);
window.opsUpdate = (board) => view.update(board);
