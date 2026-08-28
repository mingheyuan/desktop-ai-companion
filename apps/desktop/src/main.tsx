import { StrictMode, useEffect } from "react";
import { createRoot } from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { App } from "./App";
import type { AvatarSemanticState } from "./contracts";
import { getSnapshot, isTauri, listenToCoreEvents } from "./lib/tauri";
import { useAppStore } from "./stores/app";
import "./styles.css";

const previewParams = new URLSearchParams(window.location.search);
const currentWindowLabel = isTauri() ? getCurrentWindow().label : previewParams.get("window") === "pet" ? "pet" : "main";
const previewStates = new Set<AvatarSemanticState>(["idle", "listening", "thinking", "speaking", "sleeping", "needs_input", "ready", "recovering", "error"]);
document.documentElement.dataset.window = currentWindowLabel;

function Runtime() {
  const store = useAppStore();

  useEffect(() => {
    if (!isTauri()) {
      const requestedState = previewParams.get("state") as AvatarSemanticState | null;
      const requestedMouth = Number(previewParams.get("mouth") ?? 0);
      store.setSnapshot({
        ...store.snapshot,
        conversation_id: "browser-preview",
        revision: 1,
        avatar_state: requestedState && previewStates.has(requestedState) ? requestedState : "idle",
        mouth_level: Number.isFinite(requestedMouth) ? Math.min(1, Math.max(0, requestedMouth)) : 0,
      });
      return;
    }
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void getSnapshot().then((snapshot) => !disposed && store.setSnapshot(snapshot));
    void listenToCoreEvents({
      snapshot: (event) => store.setSnapshot(event.payload),
      delta: (event) => store.appendDelta(event.payload, event.turn_id),
      ended: (event) => store.finishTurn(event.payload),
      mouthLevel: (event) => store.setMouthLevel(event.payload.level),
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
