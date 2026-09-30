import { MDXProvider } from "@mdx-js/react";
import { RouterProvider } from "@tanstack/react-router";
import { Provider } from "jotai";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "virtual:tokens.css";
import "./styles.css";
import { components } from "./components/mdx";
import { router } from "./router";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Provider>
      <MDXProvider components={components}>
        <RouterProvider router={router} />
      </MDXProvider>
    </Provider>
  </StrictMode>,
);
