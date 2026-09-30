import {
  createHashHistory,
  createRootRoute,
  createRoute,
  createRouter,
} from "@tanstack/react-router";
import { pages } from "./chapters";
import { Chapter, Layout } from "./components/Layout";

const root = createRootRoute({ component: Layout, notFoundComponent: Chapter });

export const routeTree = root.addChildren(
  pages.map((page) =>
    createRoute({ getParentRoute: () => root, path: page.path, component: Chapter }),
  ),
);

// A preview hosted at an unknown path (SITE_BASE=./) routes in the hash
// instead, since only that host knows its own path.
const hashed = import.meta.env.VITE_SITE_HISTORY === "hash";

export const router = createRouter({
  routeTree,
  history: hashed ? createHashHistory() : undefined,
  // Vite's base ("/tmt/" on GitHub Pages) without its trailing slash.
  basepath: hashed ? "/" : import.meta.env.BASE_URL.replace(/\/$/, "") || "/",
});

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
