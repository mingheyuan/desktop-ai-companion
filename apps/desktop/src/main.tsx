import { StrictMode, useEffect } from "react";
import { createRoot } from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { App } from "./App";
import { getSnapshot, isTauri, listenToCoreEvents } from "./lib/tauri";
import { useAppStore } from "./stores/app";
import "./styles.css";

const currentWindowLabel = isTauri() ? getCurrentWindow().label : "main";
document.documentElement.dataset.window = currentWindowLabel;

function Runtime() {
  const store = useAppStore();

  useEffect(() => {
    if (!isTauri()) {
      store.setSnapshot({ ...store.snapshot, conversation_id: "browser-preview", revision: 1 });
      return;
    }
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void getSnapshot().then((snapshot) => !disposed && store.setSnapshot(snapshot));
    void listenToCoreEvents({
      snapshot: (event) => store.setSnapshot(event.payload),
      delta: (event) => store.appendDelta(event.payload, event.turn_id),
      ended: (event) => store.finishTurn(event.payload),
    }).then((fn) => {
      if (disposed) fn(); else unlisten = fn;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  return <App windowLabel={currentWindowLabel} />;
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Runtime />
  </StrictMode>,
);
