// Global styles first: they declare the cascade-layer order (theme, base, components,
// utilities) before any component CSS (e.g. Apps SDK `@layer components`) is injected.
import "./styles/globals.css";
import React from "react";
import ReactDOM from "react-dom/client";
import MobileApp from "./App";

const rootEl = document.getElementById("root");
if (rootEl) {
  ReactDOM.createRoot(rootEl).render(
    <React.StrictMode>
      <MobileApp />
    </React.StrictMode>,
  );
}
