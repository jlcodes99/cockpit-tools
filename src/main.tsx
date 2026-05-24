import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { initI18n } from "./i18n";
import { AppRuntimeGuard } from "./components/AppRuntimeGuard";
import CodexSwitcherApp from "./codex-switcher/CodexSwitcherApp";

void initI18n();

function RootApp() {
  const path = window.location.pathname.replace(/\/+$/, "");
  if (path === "/codex-switcher") {
    return <CodexSwitcherApp />;
  }
  return <App />;
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <AppRuntimeGuard>
      <RootApp />
    </AppRuntimeGuard>
  </React.StrictMode>,
);
