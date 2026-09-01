import { FormEvent, useEffect, useRef, useState } from "react";
import type { DataSummary, MemoryCandidate, MemoryItem, MemorySettings, MemorySource } from "../contracts";
import { clearConversationData, clearMemoryData, createMemory, deleteMemory, exportMemories, getDataSummary, getMemorySettings, getMemorySource, isTauri, listMemoryCandidates, reviewMemoryCandidate, searchMemories, updateMemory, updateMemoryCandidate, updateMemorySettings } from "../lib/tauri";

function errorMessage(reason: unknown, fallback: string) {
  if (typeof reason === "string") return reason;
  if (reason && typeof reason === "object" && "message" in reason && typeof reason.message === "string") {
    return reason.message;
  }
  return fallback;
}

function memoryScopeLabel(scope: string) {
  return scope === "relationship" ? "关系记忆" : "用户记忆";
}

function memoryOriginLabel(origin: string) {
  if (origin === "user_explicit") return "用户明确请求";
  if (origin === "model_suggested") return "模型提议（已由本地校验）";
  return "自动候选";
}

export function MemoryPanel({ onClose, onJumpToMessage }: { onClose: () => void; onJumpToMessage?: (messageId: string) => void }) {
  const [query, setQuery] = useState("");
  const [items, setItems] = useState<MemoryItem[]>([]);
  const [candidates, setCandidates] = useState<MemoryCandidate[]>([]);
  const [editingCandidate, setEditingCandidate] = useState<{ id: string; content: string } | null>(null);
  const [draft, setDraft] = useState("");
  const [editing, setEditing] = useState<{ id: string; content: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [summary, setSummary] = useState<DataSummary | null>(null);
  const [memorySettings, setMemorySettings] = useState<MemorySettings | null>(null);
  const [source, setSource] = useState<MemorySource | null>(null);
  const [exportedPath, setExportedPath] = useState("");
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

  const loadCandidates = async () => {
    if (!isTauri()) return;
    try {
      setCandidates(await listMemoryCandidates());
    } catch (reason) {
      setError(errorMessage(reason, "读取记忆候选失败。"));
    }
  };

  const loadMemorySettings = async () => {
    if (!isTauri()) return;
    try {
      setMemorySettings(await getMemorySettings());
    } catch (reason) {
      setError(errorMessage(reason, "读取记忆设置失败。"));
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
    void loadCandidates();
    void loadMemorySettings();
  }, []);

  const toggleAutomaticCandidates = async (enabled: boolean) => {
    if (!memorySettings || busy) return;
    setBusy(true);
    setError("");
    try {
      setMemorySettings(await updateMemorySettings(enabled, memorySettings.revision));
    } catch (reason) {
      setError(errorMessage(reason, "更新自动记忆设置失败。"));
      await loadMemorySettings();
    } finally {
      setBusy(false);
    }
  };

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

  const showSource = async (memory: MemoryItem) => {
    if (busy || !isTauri()) return;
    if (source?.memory_id === memory.id) {
      setSource(null);
      return;
    }
    setBusy(true);
    setError("");
    try {
      setSource(await getMemorySource(memory.id));
    } catch (reason) {
      setError(errorMessage(reason, "读取记忆来源失败。"));
    } finally {
      setBusy(false);
    }
  };

  const exportAllMemories = async () => {
    if (busy || !isTauri() || !window.confirm("将导出当前角色可见的全部长期记忆，包括已过期和敏感记忆元数据。继续吗？")) return;
    setBusy(true);
    setError("");
    setExportedPath("");
    try {
      const path = await exportMemories();
      if (path) setExportedPath(path);
    } catch (reason) {
      setError(errorMessage(reason, "导出记忆失败。"));
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

  const reviewCandidate = async (candidate: MemoryCandidate, approved: boolean, resolution?: "replace" | "keep_both") => {
    if (busy || !isTauri()) return;
    setBusy(true);
    setError("");
    try {
      await reviewMemoryCandidate(candidate.id, approved, resolution);
      if (editingCandidate?.id === candidate.id) setEditingCandidate(null);
      setCandidates((current) => current.filter((item) => item.id !== candidate.id));
      await loadSummary();
      if (approved) {
        await refresh("");
        setQuery("");
      }
    } catch (reason) {
      setError(errorMessage(reason, approved ? "保存记忆候选失败。" : "拒绝记忆候选失败。"));
      await loadCandidates();
    } finally {
      setBusy(false);
    }
  };

  const saveCandidateEdit = async (candidate: MemoryCandidate) => {
    if (!editingCandidate || editingCandidate.id !== candidate.id || !editingCandidate.content.trim() || busy || !isTauri()) return;
    setBusy(true);
    setError("");
    try {
      const updated = await updateMemoryCandidate(candidate.id, editingCandidate.content);
      setCandidates((current) => current.map((item) => item.id === updated.id ? updated : item));
      setEditingCandidate(null);
    } catch (reason) {
      setError(errorMessage(reason, "编辑记忆候选失败。"));
      await loadCandidates();
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
      await loadCandidates();
      setConfirming(null);
      if (scope === "memories") {
        setEditing(null);
        setEditingCandidate(null);
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

        <p className="memory-privacy">只有你确认过的内容才会进入长期记忆。关系记忆只绑定当前角色；发送给模型的开关在 Provider 设置中，默认关闭。</p>
        <label className="memory-policy">
          <input
            type="checkbox"
            checked={memorySettings?.automatic_candidates_enabled ?? true}
            disabled={!memorySettings || busy || !isTauri()}
            onChange={(event) => void toggleAutomaticCandidates(event.target.checked)}
          />
          <span><strong>允许自动生成记忆候选</strong><small>只生成待审核草案，不会自动写入长期记忆</small></span>
        </label>
        <input className="memory-search" value={query} onChange={(event) => setQuery(event.target.value)} placeholder="搜索已保存记忆…" />

        <form className="memory-create" onSubmit={create}>
          <textarea value={draft} onChange={(event) => setDraft(event.target.value)} maxLength={1000} rows={3} placeholder="输入希望爱丽丝记住的确切内容…" />
          <button type="submit" disabled={!draft.trim() || busy || !isTauri()}>明确保存</button>
        </form>

        {error && <p className="settings-error">{error}</p>}
        <div className="memory-list">
          {candidates.length > 0 && (
            <section className="memory-candidate-section" aria-label="待确认记忆候选">
              <div className="memory-candidate-heading">
                <div>
                  <p className="eyebrow">REVIEW QUEUE</p>
                  <h3>待确认候选</h3>
                </div>
                <span>{candidates.length} 条</span>
              </div>
              <p className="memory-candidate-hint">这些内容只会在你点击“保存到记忆”后进入长期记忆。</p>
              {candidates.map((candidate) => (
                <article className="memory-candidate-card" key={candidate.id}>
                  <span className="memory-scope-label">{memoryScopeLabel(candidate.scope)}</span>
                  {editingCandidate?.id === candidate.id ? (
                    <textarea value={editingCandidate.content} maxLength={1000} rows={3} onChange={(event) => setEditingCandidate({ id: candidate.id, content: event.target.value })} />
                  ) : (
                    <p>{candidate.content}</p>
                  )}
                  {candidate.duplicate_content && (
                    <p className="memory-candidate-duplicate">
                      与已有候选或长期记忆完全重复：{candidate.duplicate_content}。保存时不会创建第二条相同记忆。
                    </p>
                  )}
                  {candidate.conflict_content && (
                    <p className="memory-candidate-conflict">
                      与已有记忆存在事实冲突：{candidate.conflict_content}。请选择替换旧记忆，或保留两条。
                    </p>
                  )}
                  <p className="memory-candidate-reason">来源：{memoryOriginLabel(candidate.origin)} · {candidate.reason} · {candidate.kind} · {candidate.sensitivity === "normal" ? "普通" : candidate.sensitivity === "sensitive" ? "敏感（不会默认发送）" : "受限"}{candidate.expires_at_ms ? ` · 审核截止 ${new Date(candidate.expires_at_ms).toLocaleDateString()}` : ""}</p>
                  <footer>
                    <span>置信度 {Math.round(candidate.confidence * 100)}% · {new Date(candidate.created_at_ms).toLocaleDateString()}</span>
                    <div>
                      {editingCandidate?.id === candidate.id ? (
                        <>
                          <button type="button" onClick={() => setEditingCandidate(null)}>取消</button>
                          <button type="button" disabled={busy || !editingCandidate.content.trim()} onClick={() => void saveCandidateEdit(candidate)}>保存候选</button>
                        </>
                      ) : (
                        <>
                          <button type="button" disabled={busy} onClick={() => setEditingCandidate({ id: candidate.id, content: candidate.content })}>编辑</button>
                          <button type="button" className="danger" disabled={busy} onClick={() => void reviewCandidate(candidate, false)}>拒绝</button>
                          {candidate.conflict_content ? (
                            <>
                              <button type="button" disabled={busy} onClick={() => void reviewCandidate(candidate, true, "replace")}>替换旧记忆</button>
                              <button type="button" disabled={busy} onClick={() => void reviewCandidate(candidate, true, "keep_both")}>保留两条</button>
                            </>
                          ) : (
                            <button type="button" disabled={busy} onClick={() => void reviewCandidate(candidate, true)}>保存到记忆</button>
                          )}
                        </>
                      )}
                    </div>
                  </footer>
                </article>
              ))}
            </section>
          )}
          {items.length === 0 && <p className="memory-empty">{isTauri() ? (candidates.length > 0 ? "还没有已保存的长期记忆。" : "没有匹配的记忆。") : "浏览器预览不会读取本地记忆。"}</p>}
          {items.map((memory) => (
            <article className="memory-card" key={memory.id}>
              <span className="memory-scope-label">{memoryScopeLabel(memory.scope)}</span>
              {memory.valid_until_ms && memory.valid_until_ms <= Date.now() && <span className="memory-expired">已过期 · 仅在管理界面保留</span>}
              {editing?.id === memory.id ? (
                <textarea value={editing.content} maxLength={1000} rows={3} onChange={(event) => setEditing({ id: memory.id, content: event.target.value })} />
              ) : (
                <p>{memory.content}</p>
              )}
              {memory.valid_until_ms && memory.valid_until_ms > Date.now() && <p className="memory-validity">有效至 {new Date(memory.valid_until_ms).toLocaleDateString()}</p>}
              {source?.memory_id === memory.id && (
                <div className="memory-source">
                  <strong>来源</strong>
                  <p>{source.message?.canonical_text ?? (source.source_turn_id ? `原对话已清理（Turn ${source.source_turn_id}）` : "来源消息不可用")}</p>
                  {source.message && onJumpToMessage && <button type="button" onClick={() => onJumpToMessage(source.message!.id)}>定位聊天</button>}
                </div>
              )}
              <footer>
                <span>v{memory.version} · {memory.kind} · {memory.sensitivity === "normal" ? "普通" : "敏感"} · {new Date(memory.updated_at_ms).toLocaleDateString()}</span>
                <div>
                  {editing?.id === memory.id ? (
                    <>
                      <button type="button" onClick={() => setEditing(null)}>取消</button>
                      <button type="button" disabled={busy} onClick={() => saveEdit(memory)}>保存修改</button>
                    </>
                  ) : (
                    <>
                      {(memory.source_turn_id || memory.source_message_id) && <button type="button" disabled={busy} onClick={() => void showSource(memory)}>{source?.memory_id === memory.id ? "收起来源" : "查看来源"}</button>}
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
            <button type="button" disabled={busy || !isTauri()} onClick={() => void exportAllMemories()}>导出 JSON</button>
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
          {exportedPath && <p className="data-success">已导出到：{exportedPath}</p>}
          <p className="data-warning">两项操作相互独立，均保留 Provider、角色模型和桌宠设置；确认后不可撤销。</p>
        </section>
      </section>
    </div>
  );
}
