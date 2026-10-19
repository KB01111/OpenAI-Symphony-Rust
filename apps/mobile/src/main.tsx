import React from "react";
import ReactDOM from "react-dom/client";
import "./styles/globals.css";
import MobileApp from "./App";

const rootEl = document.getElementById("root");
if (rootEl) {
  ReactDOM.createRoot(rootEl).render(
    <React.StrictMode>
      <MobileApp />
    </React.StrictMode>,
  );
}
