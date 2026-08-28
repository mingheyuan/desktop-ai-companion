import { beforeEach, describe, expect, it } from "vitest";
import type { AppSnapshot } from "../contracts";
import { useAppStore } from "./app";

const snapshot: AppSnapshot = {
  revision: 3,
  listening_mode: "disabled",
  input_state: "stopped",
  turn_state: "active",
  generation_state: "streaming",
  playback_state: "idle",
  avatar_state: "thinking",
  active_turn_id: "turn-current",
  generation: 7,
  conversation_id: "conversation",
  messages: [],
  provider: {
    provider: "mock",
    endpoint: "",
    model: "mock-aris",
    has_api_key: false,
    revision: 0,
  },
  last_error: null,
};

describe("app store generation projection", () => {
  beforeEach(() => {
    useAppStore.setState({ streamingText: "", booting: false });
    useAppStore.getState().setSnapshot(snapshot);
  });

  it("appends deltas for the active turn", () => {
    useAppStore.getState().appendDelta({ text: "爱丽丝" }, "turn-current");
    expect(useAppStore.getState().streamingText).toBe("爱丽丝");
  });

  it("ignores stale turn deltas", () => {
    useAppStore.getState().appendDelta({ text: "幽灵文本" }, "turn-old");
    expect(useAppStore.getState().streamingText).toBe("");
  });

  it("clears transient text when a terminal event arrives", () => {
    useAppStore.getState().appendDelta({ text: "回答" }, "turn-current");
    useAppStore.getState().finishTurn({ status: "completed" });
    expect(useAppStore.getState().streamingText).toBe("");
  });
});
