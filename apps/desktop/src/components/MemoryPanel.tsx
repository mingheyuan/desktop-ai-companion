import { FormEvent, useEffect, useRef, useState } from "react";
import type { MemoryItem } from "../contracts";
import { createMemory, deleteMemory, isTauri, searchMemories, updateMemory } from "../lib/tauri";

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
  const requestSequence = useRef(0);

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
    } catch (reason) {
      setError(errorMessage(reason, "删除记忆失败。"));
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
      </section>
    </div>
  );
}
