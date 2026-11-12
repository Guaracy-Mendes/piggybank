import React from "react";
import { createRoot } from "react-dom/client";
import { ErrorBoundary, SessionProvider } from "@piggybank/web-core";
import { Root } from "./App";
import "@piggybank/web-core/style.css";
import "./style.css";
createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <ErrorBoundary>
      <SessionProvider trust="workforce">
        <Root />
      </SessionProvider>
    </ErrorBoundary>
  </React.StrictMode>,
);
