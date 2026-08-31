import { characterModelUrl, openPanel } from "../lib/tauri";
import type { ChatMessage } from "../contracts";
import { useAppStore } from "../stores/app";
import { VrmCanvas } from "../avatar/VrmCanvas";

export interface PetSubtitle {
  previous: string | null;
  current: string;
}

const SENTENCE_ENDINGS = new Set(["。", "！", "？", "；", ".", "!", "?", ";", "\n"]);

export function splitSubtitleSentences(text: string): string[] {
  const normalized = text.replace(/\s+/g, " ").trim();
  if (!normalized) return [];

  const sentences: string[] = [];
  let current = "";
  for (const character of normalized) {
    current += character;
    if (SENTENCE_ENDINGS.has(character)) {
      const sentence = current.trim();
      if (sentence) sentences.push(sentence);
      current = "";
    }
  }
  const trailing = current.trim();
  if (trailing) sentences.push(trailing);
  return sentences;
}

export function buildPetSubtitle(messages: ChatMessage[], streamingText: string): PetSubtitle {
  const completedAssistantSentences = messages
    .filter((message) => message.role === "assistant" && message.canonical_text.trim())
    .flatMap((message) => splitSubtitleSentences(message.canonical_text));
  const activeSentences = splitSubtitleSentences(streamingText);

  if (activeSentences.length > 0) {
    return {
      previous: activeSentences.at(-2) ?? completedAssistantSentences.at(-1) ?? null,
      current: activeSentences.at(-1) ?? "爱丽丝正在思考…",
    };
  }

  return {
    previous: completedAssistantSentences.at(-2) ?? null,
    current: completedAssistantSentences.at(-1) ?? "老师，爱丽丝已就绪。",
  };
}

export function PetWindow() {
  const { snapshot, streamingText } = useAppStore();
  const subtitle = buildPetSubtitle(snapshot.messages, streamingText);
  const modelUrl = characterModelUrl(snapshot.character);

  return (
    <main className="pet-shell">
      <button className="pet-drag-region" data-tauri-drag-region aria-label="拖动桌宠窗口" />
      <button className="speech-bubble" onClick={() => openPanel()} aria-label="打开聊天窗口" aria-live="polite">
        {subtitle.previous && <span className="speech-line speech-line-previous">{subtitle.previous}</span>}
        <span className="speech-line speech-line-current">{subtitle.current}</span>
      </button>
      <button
        className="avatar-button"
        data-tauri-drag-region
        onClick={() => openPanel()}
        aria-label="单击打开聊天，拖动移动桌宠"
        title="单击聊天 · 拖动移动"
      >
        <VrmCanvas state={snapshot.avatar_state} mouthLevel={snapshot.mouth_level} modelUrl={modelUrl} />
      </button>
      <span className={`pet-indicator state-${snapshot.avatar_state}`} />
    </main>
  );
}
