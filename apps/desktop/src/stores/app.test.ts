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
    allow_memory_context: false,
    revision: 0,
  },
  pet: {
    visible: true,
    click_through: false,
    scale: 1,
    revision: 0,
  },
  character: {
    character_id: "aris",
    display_name: "爱丽丝",
    model_path: null,
    model_file_name: null,
    model_size_bytes: null,
    revision: 0,
  },
  audio_host_state: "stopped",
  voice_permissions: {
    microphone: "not_determined",
    speech_recognition: "not_determined",
  },
  voice_settings: {
    tts_enabled: false,
    revision: 0,
  },
    asr_partial: "",
    voice_input_route: "unknown",
    mouth_level: 0,
    pending_tool_approval: null,
    timers: [],
    last_error: null,
};

describe("app store generation projection", () => {
  beforeEach(() => {
    useAppStore.setState({ snapshot, streamingText: "", booting: false });
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
    useAppStore.getState().finishTurn({ status: "completed" }, "turn-current");
    expect(useAppStore.getState().streamingText).toBe("");
  });

  it("ignores a stale terminal event for an older turn", () => {
    useAppStore.getState().appendDelta({ text: "新回答" }, "turn-current");
    useAppStore.getState().finishTurn({ status: "cancelled" }, "turn-old");
    expect(useAppStore.getState().streamingText).toBe("新回答");
  });

  it("projects transient mouth level without replacing the snapshot", () => {
    useAppStore.getState().setMouthLevel(0.72, 7);
    expect(useAppStore.getState().snapshot.mouth_level).toBe(0.72);
    expect(useAppStore.getState().snapshot.active_turn_id).toBe("turn-current");
  });

  it("ignores mouth animation from an older generation", () => {
    useAppStore.getState().setMouthLevel(0.72, 6);
    expect(useAppStore.getState().snapshot.mouth_level).toBe(0);
  });

  it("does not let a late bootstrap snapshot replace newer state", () => {
    useAppStore.getState().setSnapshot({ ...snapshot, revision: 8, avatar_state: "speaking" });
    useAppStore.getState().setSnapshot({ ...snapshot, revision: 7, avatar_state: "idle" });
    expect(useAppStore.getState().snapshot.revision).toBe(8);
    expect(useAppStore.getState().snapshot.avatar_state).toBe("speaking");
  });

  it("merges desktop settings without replacing a newer snapshot", () => {
    useAppStore.getState().setSnapshot({ ...snapshot, revision: 8, messages: [{
      id: "m1",
      turn_id: "turn-current",
      role: "user",
      canonical_text: "保留我",
      status: "completed",
      created_at_ms: 1,
    }] });
    useAppStore.getState().setPet({ ...snapshot.pet, visible: false, revision: 1 });
    const state = useAppStore.getState().snapshot;
    expect(state.revision).toBe(8);
    expect(state.messages[0].canonical_text).toBe("保留我");
    expect(state.pet.visible).toBe(false);
  });
});
