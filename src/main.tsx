import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { initI18n } from "./i18n";
import { AppRuntimeGuard } from "./components/AppRuntimeGuard";
import {
  captureError,
  initErrorReporter,
  markFrontendReady,
  recordFrontendStage,
} from "./utils/errorReporter";
import { setBootSplashStage } from "./utils/bootSplash";

initErrorReporter();
recordFrontendStage("script_loaded");
setBootSplashStage("script_loaded");
void initI18n();

const rootElement = document.getElementById("root");
if (!rootElement) {
  const error = new Error("Root element not found");
  captureError(error, { source: "frontend_boot", phase: "root_lookup" });
  throw error;
}

recordFrontendStage("react_mount_start");
ReactDOM.createRoot(rootElement).render(
  <React.StrictMode>
    <AppRuntimeGuard>
      <App />
    </AppRuntimeGuard>
  </React.StrictMode>,
);

window.requestAnimationFrame(() => {
  setBootSplashStage("react_mounted");
  markFrontendReady("react_mounted");
});
