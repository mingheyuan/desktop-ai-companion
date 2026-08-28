import { FormEvent, useEffect, useRef, useState } from "react";
import type { DataSummary, MemoryItem } from "../contracts";
import { clearConversationData, clearMemoryData, createMemory, deleteMemory, getDataSummary, isTauri, searchMemories, updateMemory } from "../lib/tauri";

function errorMessage(reason: unknown, fallback: string) {
  if (typeof reason === "string") return reason;
  if (reason && typeof reason === "object" && "message" in reason && typeof reason.message === "string") {
    return reason.message;
  }
  return fallback;
}

export function MemoryPanel({ onClose }: { onClose: () => void }) {
  const [query, setQuery] = useState("");
  const [items, setItems] = useState<MemoryItem[]>([]);
  const [draft, setDraft] = useState("");
  const [editing, setEditing] = useState<{ id: string; content: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [summary, setSummary] = useState<DataSummary | null>(null);
  const [confirming, setConfirming] = useState<"conversation" | "memories" | null>(null);
  const requestSequence = useRef(0);

  const loadSummary = async () => {
    if (!isTauri()) return;
    try {
      setSummary(await getDataSummary());
    } catch (reason) {
      setError(errorMessage(reason, "读取本地数据摘要失败。"));
    }
  };

  const refresh = async (search = query) => {
    if (!isTauri()) return;
    const sequence = ++requestSequence.current;
    try {
      const memories = await searchMemories(search, 20);
      if (sequence === requestSequence.current) setItems(memories);
    } catch (reason) {
      if (sequence === requestSequence.current) setError(errorMessage(reason, "读取记忆失败。"));
    }
  };

  useEffect(() => {
    const timer = window.setTimeout(() => void refresh(query), 180);
    return () => window.clearTimeout(timer);
  }, [query]);

  useEffect(() => {
    void loadSummary();
  }, []);

  const create = async (event: FormEvent) => {
    event.preventDefault();
    const content = draft.trim();
    if (!content || busy || !isTauri()) return;
    setBusy(true);
    setError("");
    try {
      await createMemory(content);
      setDraft("");
      await refresh("");
      await loadSummary();
      setQuery("");
    } catch (reason) {
      setError(errorMessage(reason, "保存记忆失败。"));
    } finally {
      setBusy(false);
    }
  };

  const saveEdit = async (memory: MemoryItem) => {
    if (!editing?.content.trim() || busy) return;
    setBusy(true);
    setError("");
    try {
      await updateMemory(memory, editing.content.trim());
      setEditing(null);
      await refresh();
    } catch (reason) {
      setError(errorMessage(reason, "更新记忆失败。"));
    } finally {
      setBusy(false);
    }
  };

  const remove = async (memory: MemoryItem) => {
    if (busy || !window.confirm("确定永久删除这条记忆吗？正文会从本地索引中移除。")) return;
    setBusy(true);
    setError("");
    try {
      await deleteMemory(memory);
      await refresh();
      await loadSummary();
    } catch (reason) {
      setError(errorMessage(reason, "删除记忆失败。"));
    } finally {
      setBusy(false);
    }
  };

  const clearData = async (scope: "conversation" | "memories") => {
    if (busy || !isTauri()) return;
    setBusy(true);
    setError("");
    try {
      const nextSummary = scope === "conversation" ? await clearConversationData() : await clearMemoryData();
      setSummary(nextSummary);
      setConfirming(null);
      if (scope === "memories") {
        setEditing(null);
        await refresh("");
        setQuery("");
      }
    } catch (reason) {
      setError(errorMessage(reason, scope === "conversation" ? "清空聊天记录失败。" : "清空长期记忆失败。"));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="dialog-backdrop" role="presentation" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <section className="memory-dialog" aria-label="记忆管理">
        <header>
          <div>
            <p className="eyebrow">LOCAL MEMORY</p>
            <h2>爱丽丝的记忆</h2>
          </div>
          <button type="button" className="icon-button" onClick={onClose} aria-label="关闭记忆管理">×</button>
        </header>

        <p className="memory-privacy">只有你在这里明确保存的内容才会进入长期记忆。发送给模型的开关在 Provider 设置中，默认关闭。</p>
        <input className="memory-search" value={query} onChange={(event) => setQuery(event.target.value)} placeholder="搜索已保存记忆…" />

        <form className="memory-create" onSubmit={create}>
          <textarea value={draft} onChange={(event) => setDraft(event.target.value)} maxLength={1000} rows={3} placeholder="输入希望爱丽丝记住的确切内容…" />
          <button type="submit" disabled={!draft.trim() || busy || !isTauri()}>明确保存</button>
        </form>

        {error && <p className="settings-error">{error}</p>}
        <div className="memory-list">
          {items.length === 0 && <p className="memory-empty">{isTauri() ? "没有匹配的记忆。" : "浏览器预览不会读取本地记忆。"}</p>}
          {items.map((memory) => (
            <article className="memory-card" key={memory.id}>
              {editing?.id === memory.id ? (
                <textarea value={editing.content} maxLength={1000} rows={3} onChange={(event) => setEditing({ id: memory.id, content: event.target.value })} />
              ) : (
                <p>{memory.content}</p>
              )}
              <footer>
                <span>v{memory.version} · {new Date(memory.updated_at_ms).toLocaleDateString()}</span>
                <div>
                  {editing?.id === memory.id ? (
                    <>
                      <button type="button" onClick={() => setEditing(null)}>取消</button>
                      <button type="button" disabled={busy} onClick={() => saveEdit(memory)}>保存修改</button>
                    </>
                  ) : (
                    <>
                      <button type="button" onClick={() => setEditing({ id: memory.id, content: memory.content })}>编辑</button>
                      <button type="button" className="danger" disabled={busy} onClick={() => remove(memory)}>删除</button>
                    </>
                  )}
                </div>
              </footer>
            </article>
          ))}
        </div>

        <section className="data-management" aria-label="本地数据清理">
          <div>
            <h3>本地数据</h3>
            <p>聊天 {summary?.message_count ?? "—"} 条 · 长期记忆 {summary?.memory_count ?? "—"} 条 · 待确认候选 {summary?.memory_candidate_count ?? "—"} 条</p>
          </div>
          <div className="data-actions">
            {confirming === "conversation" ? (
              <span className="confirm-actions">
                <button type="button" onClick={() => setConfirming(null)}>取消</button>
                <button type="button" className="danger" disabled={busy} onClick={() => void clearData("conversation")}>确认清空聊天</button>
              </span>
            ) : (
              <button type="button" disabled={busy || !isTauri() || !summary || summary.message_count === 0} onClick={() => setConfirming("conversation")}>清空聊天记录</button>
            )}
            {confirming === "memories" ? (
              <span className="confirm-actions">
                <button type="button" onClick={() => setConfirming(null)}>取消</button>
                <button type="button" className="danger" disabled={busy} onClick={() => void clearData("memories")}>确认清空记忆</button>
              </span>
            ) : (
              <button type="button" className="danger" disabled={busy || !isTauri() || !summary || (summary.memory_count === 0 && summary.memory_candidate_count === 0)} onClick={() => setConfirming("memories")}>清空全部记忆</button>
            )}
          </div>
          <p className="data-warning">两项操作相互独立，均保留 Provider、角色模型和桌宠设置；确认后不可撤销。</p>
        </section>
      </section>
    </div>
  );
}
