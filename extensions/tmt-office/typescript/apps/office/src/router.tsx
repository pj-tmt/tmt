import {
  createRootRoute,
  createRoute,
  createRouter,
  Link,
  useLocation,
} from '@tanstack/react-router';
import type { RouterHistory } from '@tanstack/react-router';
import { OfficeShell } from './shell.js';
import { HomePage } from './pages/home.js';
import { SetupPage } from './pages/setup.js';
import { SelectedWorld, WorldGate } from './worlds/world-view.js';
import { PairingPanel } from './pairing/pairing-view.js';
import { LocalOfficePage } from './local/local-page.js';
import { LocalBoardPage } from './local/board-page.js';
import { PropPreviewPage } from './props/preview-page.js';
import { AvatarPreviewPage } from './avatars/preview-page.js';
import { LocalWhiteboardPage } from './whiteboard/editor.js';

const rootRoute = createRootRoute({
  component: OfficeShell,
  notFoundComponent: () => (
    <section>
      <h1>Page not found</h1>
      <p>This address is not part of the Office preview.</p>
      <Link to="/">Return to Office</Link>
    </section>
  ),
});

const worldRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/worlds/$worldId',
  component: WorldPage,
});
function WorldPage() {
  const { worldId } = worldRoute.useParams();
  return <WorldGate>{(state) => <SelectedWorld state={state} id={worldId} />}</WorldGate>;
}

const pairingRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/worlds/$worldId/pair',
  component: PairingPage,
});
const localRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/local',
  component: LocalOfficePage,
});
const localBoardRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/local/board',
  component: LocalBoardPage,
});
const localWhiteboardRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/local/whiteboards/$documentId',
  component: LocalWhiteboardRoute,
});
function LocalWhiteboardRoute() {
  const { documentId } = localWhiteboardRoute.useParams();
  return <LocalWhiteboardPage documentId={documentId} />;
}
const localSpaceRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/local/agents/$identityId',
  component: LocalSpaceRoute,
});
function LocalSpaceRoute() {
  const { identityId } = localSpaceRoute.useParams();
  return <LocalOfficePage key={identityId} initialIdentityId={identityId} />;
}
const localLobbyRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/local/lobby',
  component: LocalOfficePage,
});
const localPropPreviewRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/local/props/preview/$previewId',
  component: LocalPropPreview,
});
function LocalPropPreview() {
  const { previewId } = localPropPreviewRoute.useParams();
  return <PropPreviewPage previewId={previewId} />;
}
const localAvatarPreviewRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/local/avatars/preview/$previewId',
  component: LocalAvatarPreview,
});
function LocalAvatarPreview() {
  const { previewId } = localAvatarPreviewRoute.useParams();
  return <AvatarPreviewPage previewId={previewId} />;
}
function PairingPage() {
  const { worldId } = pairingRoute.useParams();
  const fragment = useLocation({ select: (location) => location.hash });
  return (
    <WorldGate>
      {(state) => (
        <SelectedWorld state={state} id={worldId}>
          {(world) => (
            <PairingPanel worldId={world.id} ownerUid={world.ownerUid} fragment={fragment} />
          )}
        </SelectedWorld>
      )}
    </WorldGate>
  );
}

const routeTree = rootRoute.addChildren([
  createRoute({ getParentRoute: () => rootRoute, path: '/', component: HomePage }),
  createRoute({ getParentRoute: () => rootRoute, path: '/setup', component: SetupPage }),
  worldRoute,
  pairingRoute,
  localRoute,
  localSpaceRoute,
  localLobbyRoute,
  localBoardRoute,
  localWhiteboardRoute,
  localPropPreviewRoute,
  localAvatarPreviewRoute,
]);

export function createOfficeRouter(history?: RouterHistory) {
  return createRouter({ routeTree, history });
}

declare module '@tanstack/react-router' {
  interface Register {
    router: ReturnType<typeof createOfficeRouter>;
  }
}
