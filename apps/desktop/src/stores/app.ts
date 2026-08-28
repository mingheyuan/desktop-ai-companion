import { create } from "zustand";
import type { AppSnapshot, CharacterSettings, PetSettings, ProviderSettings, TextDeltaPayload, TurnEndedPayload, VoiceSettings } from "../contracts";

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
  last_error: null,
};

interface AppStore {
  snapshot: AppSnapshot;
  streamingText: string;
  booting: boolean;
  setSnapshot: (snapshot: AppSnapshot) => void;
  setProvider: (provider: ProviderSettings) => void;
  setPet: (pet: PetSettings) => void;
  setCharacter: (character: CharacterSettings) => void;
  setVoiceSettings: (settings: VoiceSettings) => void;
  setMouthLevel: (level: number) => void;
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
  setProvider: (provider) =>
    set((state) => ({ snapshot: { ...state.snapshot, provider } })),
  setPet: (pet) =>
    set((state) => ({ snapshot: { ...state.snapshot, pet } })),
  setCharacter: (character) =>
    set((state) => ({ snapshot: { ...state.snapshot, character } })),
  setVoiceSettings: (voice_settings) =>
    set((state) => ({ snapshot: { ...state.snapshot, voice_settings } })),
  setMouthLevel: (mouth_level) =>
    set((state) => ({ snapshot: { ...state.snapshot, mouth_level } })),
  appendDelta: ({ text }, turnId) =>
    set((state) => {
      if (turnId && turnId !== state.snapshot.active_turn_id) return state;
      return { streamingText: state.streamingText + text };
    }),
  finishTurn: () => set({ streamingText: "" }),
}));
