import { create } from "zustand";
import type { AppSnapshot, TextDeltaPayload, TurnEndedPayload } from "../contracts";

const emptySnapshot: AppSnapshot = {
  revision: 0,
  listening_mode: "disabled",
  input_state: "stopped",
  turn_state: "idle",
  generation_state: "not_started",
  playback_state: "idle",
  avatar_state: "idle",
  active_turn_id: null,
  generation: 0,
  conversation_id: "",
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

interface AppStore {
  snapshot: AppSnapshot;
  streamingText: string;
  booting: boolean;
  setSnapshot: (snapshot: AppSnapshot) => void;
  appendDelta: (payload: TextDeltaPayload, turnId?: string) => void;
  finishTurn: (payload: TurnEndedPayload) => void;
}

export const useAppStore = create<AppStore>((set) => ({
  snapshot: emptySnapshot,
  streamingText: "",
  booting: true,
  setSnapshot: (snapshot) =>
    set((state) => ({
      snapshot,
      streamingText:
        snapshot.turn_state === "active" && snapshot.active_turn_id === state.snapshot.active_turn_id
          ? state.streamingText
          : "",
      booting: false,
    })),
  appendDelta: ({ text }, turnId) =>
    set((state) => {
      if (turnId && turnId !== state.snapshot.active_turn_id) return state;
      return { streamingText: state.streamingText + text };
    }),
  finishTurn: () => set({ streamingText: "" }),
}));
