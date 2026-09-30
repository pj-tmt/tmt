import { createRootRoute, createRoute, createRouter } from "@tanstack/react-router";
import { pages } from "./chapters";
import { Chapter, Layout } from "./components/Layout";

const root = createRootRoute({ component: Layout, notFoundComponent: Chapter });

export const routeTree = root.addChildren(
  pages.map((page) =>
    createRoute({ getParentRoute: () => root, path: page.path, component: Chapter }),
  ),
);

export const router = createRouter({
  routeTree,
  // Vite's base ("/tmt/" on GitHub Pages) without its trailing slash.
  basepath: import.meta.env.BASE_URL.replace(/\/$/, "") || "/",
});

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
