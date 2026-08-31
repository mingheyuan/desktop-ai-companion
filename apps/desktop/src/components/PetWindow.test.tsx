import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { useAppStore } from "../stores/app";
import { buildPetSubtitle, PetWindow, splitSubtitleSentences } from "./PetWindow";

describe("PetWindow subtitles", () => {
  it("splits Chinese and Latin sentence boundaries while keeping punctuation", () => {
    expect(splitSubtitleSentences("第一句。第二句！ Third sentence?"))
      .toEqual(["第一句。", "第二句！", "Third sentence?"]);
  });

  it("shows the current streaming sentence and exactly one previous sentence", () => {
    const messages = [{
      id: "assistant-1",
      turn_id: "turn-1",
      role: "assistant" as const,
      canonical_text: "上一轮已经完成。",
      status: "completed" as const,
      created_at_ms: 1,
    }];
    expect(buildPetSubtitle(messages, "当前第一句。当前第二句正在生成"))
      .toEqual({ previous: "当前第一句。", current: "当前第二句正在生成" });
  });

  it("uses the last two completed sentences when streaming is idle", () => {
    const messages = [{
      id: "assistant-1",
      turn_id: "turn-1",
      role: "assistant" as const,
      canonical_text: "第一句。第二句。第三句。",
      status: "completed" as const,
      created_at_ms: 1,
    }];
    expect(buildPetSubtitle(messages, ""))
      .toEqual({ previous: "第二句。", current: "第三句。" });
  });
});

describe("PetWindow interaction surface", () => {
  it("makes the avatar itself draggable and exposes single-click chat semantics", () => {
    useAppStore.setState((state) => ({
      ...state,
      booting: false,
      streamingText: "",
      snapshot: {
        ...state.snapshot,
        revision: 1,
        conversation_id: "test",
        character: { ...state.snapshot.character, model_path: null },
      },
    }));

    const markup = renderToStaticMarkup(<PetWindow />);
    expect(markup).toContain('class="avatar-button"');
    expect(markup).toContain('data-tauri-drag-region="true"');
    expect(markup).toContain('aria-label="单击打开聊天，拖动移动桌宠"');
  });
});
