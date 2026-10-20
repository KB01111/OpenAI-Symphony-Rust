import React from "react";
import { AppProviders } from "./AppProviders.js";
import { AppRouter } from "./AppRouter.js";
import { PreferencesBridge } from "./PreferencesBridge.js";
import { RuntimeGate } from "./RuntimeGate.js";

export const App: React.FC = () => (
  <AppProviders>
    <RuntimeGate>
      <PreferencesBridge />
      <AppRouter />
    </RuntimeGate>
  </AppProviders>
);
