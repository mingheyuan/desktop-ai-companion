import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { useAppStore } from "../stores/app";
import { PetWindow } from "./PetWindow";

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
