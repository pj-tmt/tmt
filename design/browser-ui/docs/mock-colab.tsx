import { TopBar } from './app.js';

/** The Colab mockup (#5821): filled in next; the route exists so the top bar can link to it. */
export function ColabMock({ path }: { path: string }) {
  return (
    <div className="docs">
      <TopBar />
      <main className="docs-main">
        <p className="docs-muted">The Colab mock ({path}) is being built.</p>
      </main>
    </div>
  );
}
