import { characterModelUrl, openPanel } from "../lib/tauri";
import { useAppStore } from "../stores/app";
import { VrmCanvas } from "../avatar/VrmCanvas";

export function PetWindow() {
  const { snapshot, streamingText } = useAppStore();
  const lastAssistant = [...snapshot.messages].reverse().find((message) => message.role === "assistant");
  const bubble = streamingText || lastAssistant?.canonical_text || "老师，爱丽丝已就绪。";
  const modelUrl = characterModelUrl(snapshot.character);

  return (
    <main className="pet-shell">
      <button className="pet-drag-region" data-tauri-drag-region aria-label="拖动桌宠窗口" />
      <button className="speech-bubble" onClick={() => openPanel()}>{bubble}</button>
      <button className="avatar-button" onDoubleClick={() => openPanel()} aria-label="打开聊天面板">
        <VrmCanvas state={snapshot.avatar_state} mouthLevel={snapshot.mouth_level} modelUrl={modelUrl} />
      </button>
      <span className={`pet-indicator state-${snapshot.avatar_state}`} />
    </main>
  );
}
