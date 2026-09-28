import { useCallback, useMemo, useState } from 'react';
import { ExternalLinkReview } from '../extensions/external-link-review.js';
import { BUNDLED_EXTENSIONS } from '../extensions/bundled-extensions.js';
import { activateExtension, bindWorldExtension } from '../extensions/extension-binding.js';
import type { WorldDocument } from '../world-map/world-contract.js';
import type { CatalogPack } from '../props/prop-contract.js';
import { useDiscussionBoard } from './discussion-board-entry.js';
import type { SceneAction } from '../rendering/scene-component-geometry.js';
import { useExtensionPanel } from '../extensions/use-extension-panel.js';
import { useWhiteboardPanel } from '../whiteboard/use-whiteboard-panel.js';
import { Broadcaster } from './broadcaster.js';
import { worldExtensionGroups } from '../extensions/world-extension-groups.js';
import { useNotebookPanel } from '../notebooks/use-notebook-panel.js';

/** Local composition root: bundled definitions use the same guarded binding path. */
export function useWorldExtensions(world: WorldDocument, catalog: CatalogPack[]) {
  const discussion = useDiscussionBoard();
  const whiteboard = useWhiteboardPanel();
  const notebook = useNotebookPanel();
  const broadcaster = useExtensionPanel('Broadcast station', <Broadcaster />, 'broadcaster-panel');
  const [destination, setDestination] = useState('');
  const link = useExtensionPanel(
    'Web destination',
    <ExternalLinkReview destination={destination} />,
    'external-link-panel'
  );
  const openDiscussion = discussion.open;
  const openWhiteboard = whiteboard.open;
  const openNotebook = notebook.open;
  const openBroadcaster = broadcaster.open;
  const openLink = link.open;
  const [focused, focus] = useState<string>();
  const entries = useMemo(
    () =>
      world.objects.flatMap((object) => {
        const entry = bindWorldExtension(object, BUNDLED_EXTENSIONS, catalog, {
          'discussion.open': openDiscussion,
          'broadcast.open': openBroadcaster,
          'link.open': (binding) => {
            if (binding.kind !== 'external-link') return;
            setDestination(binding.url);
            openLink();
          },
          'whiteboard.open': (binding) => {
            if (binding.kind !== 'whiteboard') return;
            openWhiteboard(binding.documentId);
          },
          'notebook.open': (binding) => {
            if (binding.kind === 'notebook') openNotebook(binding.identityId);
          },
        });
        return entry ? [entry] : [];
      }),
    [world, catalog, openDiscussion, openWhiteboard, openBroadcaster, openLink, openNotebook]
  );
  const activate = useCallback(
    (id: string) => {
      activateExtension(entries, id);
    },
    [entries]
  );
  const groups = useMemo(() => worldExtensionGroups(world, entries), [world, entries]);
  const components = useMemo<SceneAction[]>(
    () =>
      entries.map((entry) => ({
        id: entry.instance.id,
        label: entry.actionLabel ?? entry.label,
        available: Boolean(entry.invoke) && !entry.unavailable,
      })),
    [entries]
  );
  const panel = (
    <>
      {discussion.panel}
      {whiteboard.panel}
      {broadcaster.panel}
      {link.panel}
      {notebook.panel}
    </>
  );
  return { groups, components, activate, panel, focused, focus };
}
