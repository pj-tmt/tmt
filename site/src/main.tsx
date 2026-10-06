import { RouterProvider } from "@tanstack/react-router";
import { Provider } from "jotai";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "virtual:tokens.css";
import "./styles.css";
import { router } from "./router";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Provider>
      <RouterProvider router={router} />
    </Provider>
  </StrictMode>,
);

if (import.meta.env.DEV && import.meta.env.VITE_COLAB_EMBED === "1") {
  void import("./components/colab-embed/ColabEmbed").then(({ mountColabEmbed }) =>
    mountColabEmbed(),
  );
}
