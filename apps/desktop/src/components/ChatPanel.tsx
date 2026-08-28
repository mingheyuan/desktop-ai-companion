import { FormEvent, useState } from "react";
import { cancelTurn, submitText } from "../lib/tauri";
import { useAppStore } from "../stores/app";

export function ChatPanel() {
  const { snapshot, streamingText, booting } = useAppStore();
  const [draft, setDraft] = useState("");
  const [submitting, setSubmitting] = useState(false);

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
        <div className={`status-pill state-${snapshot.avatar_state}`}>{snapshot.avatar_state}</div>
      </header>

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
    </main>
  );
}
