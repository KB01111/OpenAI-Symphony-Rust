// Global styles first: they declare the cascade-layer order (theme, base, components,
// utilities) before any component CSS (e.g. Apps SDK `@layer components`) is injected.
import "./styles/globals.css";
import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "./app/App.js";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
