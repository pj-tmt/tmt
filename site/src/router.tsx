import {
  createHashHistory,
  createRootRoute,
  createRoute,
  createRouter,
  redirect,
} from "@tanstack/react-router";
import { Layout } from "./components/Layout";
import { languages, splitLang, withLang } from "./lang/languages";

const homeAnchors = new Set([
  "top",
  "install",
  "workflow",
  "squad",
  "squad-demo",
  "colab",
  "colab-demo",
  "start",
]);

const root = createRootRoute({
  component: Layout,
  notFoundComponent: Layout,
  beforeLoad: ({ location }) => {
    const pathname = location.pathname.replace(/^\/zh(?=\/|$)/, "/zh-hant");
    const { lang, path } = splitLang(pathname);
    const home = withLang(lang, "/");
    if (path !== "/" || pathname !== location.pathname) {
      throw redirect({
        to: home,
        hash:
          path === "/" && homeAnchors.has(location.hash)
            ? location.hash
            : path === "/extensions/squad"
              ? "squad"
              : path === "/extensions/colab"
                ? "colab"
                : undefined,
        search: location.search,
        replace: true,
      });
    }
  },
});

export const routeTree = root.addChildren(
  languages.map((language) =>
    createRoute({ getParentRoute: () => root, path: withLang(language.code, "/") }),
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
