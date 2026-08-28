import { FormEvent, useState } from "react";
import { cancelTurn, importCharacterVrm, isTauri, submitText, updatePetSettings } from "../lib/tauri";
import { useAppStore } from "../stores/app";
import { ProviderSettingsDialog } from "./ProviderSettingsDialog";

export function ChatPanel() {
  const { snapshot, streamingText, booting, setProvider, setPet, setCharacter } = useAppStore();
  const [draft, setDraft] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [petUpdating, setPetUpdating] = useState(false);
  const [petError, setPetError] = useState("");
  const [importingVrm, setImportingVrm] = useState(false);

  const importVrm = async () => {
    if (importingVrm || !isTauri()) return;
    setImportingVrm(true);
    setPetError("");
    try {
      const character = await importCharacterVrm();
      if (character) setCharacter(character);
    } catch (reason) {
      if (typeof reason === "string") {
        setPetError(reason);
      } else if (reason && typeof reason === "object" && "message" in reason && typeof reason.message === "string") {
        setPetError(reason.message);
      } else {
        setPetError("导入 VRM 角色失败。");
      }
    } finally {
      setImportingVrm(false);
    }
  };

  const patchPet = async (patch: Partial<typeof snapshot.pet>) => {
    if (petUpdating) return;
    setPetUpdating(true);
    setPetError("");
    try {
      const pet = await updatePetSettings({
        visible: patch.visible ?? snapshot.pet.visible,
        click_through: patch.click_through ?? snapshot.pet.click_through,
        scale: patch.scale ?? snapshot.pet.scale,
        expected_revision: snapshot.pet.revision,
      });
      setPet(pet);
    } catch (reason) {
      if (typeof reason === "string") {
        setPetError(reason);
      } else if (reason && typeof reason === "object" && "message" in reason && typeof reason.message === "string") {
        setPetError(reason.message);
      } else {
        setPetError("更新桌宠窗口失败。");
      }
    } finally {
      setPetUpdating(false);
    }
  };

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    const text = draft.trim();
    if (!text || submitting) return;
    setDraft("");
    setSubmitting(true);
    try {
      await submitText(text);
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <main className="panel-shell">
      <header className="panel-header">
        <div>
          <p className="eyebrow">DESKTOP AI COMPANION</p>
          <h1>爱丽丝</h1>
        </div>
        <div className="header-actions">
          <span className="provider-badge">{snapshot.provider.provider === "mock" ? "MOCK" : snapshot.provider.model}</span>
          <div className={`status-pill state-${snapshot.avatar_state}`}>{snapshot.avatar_state}</div>
          <button className="icon-button" type="button" onClick={() => setSettingsOpen(true)} aria-label="打开设置">⚙</button>
        </div>
      </header>

      <nav className="pet-controls" aria-label="桌宠窗口控制">
        <button type="button" disabled={petUpdating} onClick={() => patchPet({ visible: !snapshot.pet.visible })}>
          {snapshot.pet.visible ? "隐藏桌宠" : "显示桌宠"}
        </button>
        <button
          type="button"
          className={snapshot.pet.click_through ? "active" : ""}
          disabled={petUpdating}
          onClick={() => patchPet({ click_through: !snapshot.pet.click_through })}
        >
          {snapshot.pet.click_through ? "恢复桌宠交互" : "开启点击穿透"}
        </button>
        <label>
          桌宠大小
          <select
            value={snapshot.pet.scale}
            disabled={petUpdating}
            onChange={(event) => patchPet({ scale: Number(event.target.value) })}
          >
            <option value={0.8}>80%</option>
            <option value={1}>100%</option>
            <option value={1.2}>120%</option>
            <option value={1.5}>150%</option>
          </select>
        </label>
        <button type="button" disabled={importingVrm || !isTauri()} onClick={importVrm}>
          {importingVrm ? "导入中…" : snapshot.character.model_file_name ?? "导入 VRM"}
        </button>
        {petError && <span className="pet-control-error">{petError}</span>}
        <span className="pet-shortcut-hint">快捷键：⌘/Ctrl + Shift + Space</span>
      </nav>

      <section className="conversation" aria-live="polite">
        {booting && <p className="empty-state">正在连接 Rust Core…</p>}
        {!booting && snapshot.messages.length === 0 && !streamingText && (
          <div className="empty-state">
            <span className="pixel-mark">◆</span>
            <p>冒险尚未开始。老师，输入一句话试试吧。</p>
          </div>
        )}
        {snapshot.messages.map((message) => (
          <article className={`message message-${message.role}`} key={message.id}>
            <span>{message.role === "user" ? "老师" : "爱丽丝"}</span>
            <p>{message.canonical_text}</p>
          </article>
        ))}
        {streamingText && (
          <article className="message message-assistant streaming">
            <span>爱丽丝</span>
            <p>{streamingText}<i className="caret" /></p>
          </article>
        )}
      </section>

      {snapshot.last_error && <div className="error-banner">{snapshot.last_error.message}</div>}

      <form className="composer" onSubmit={submit}>
        <textarea
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.shiftKey) {
              event.preventDefault();
              event.currentTarget.form?.requestSubmit();
            }
          }}
          placeholder="和爱丽丝说点什么…"
          rows={2}
        />
        <div className="composer-actions">
          {snapshot.active_turn_id && snapshot.turn_state !== "completed" && (
            <button type="button" className="secondary" onClick={() => cancelTurn(snapshot.active_turn_id!)}>停止</button>
          )}
          <button type="submit" disabled={!draft.trim() || submitting}>发送</button>
        </div>
      </form>
      {settingsOpen && (
        <ProviderSettingsDialog
          initial={snapshot.provider}
          onClose={() => setSettingsOpen(false)}
          onSaved={setProvider}
        />
      )}
    </main>
  );
}
