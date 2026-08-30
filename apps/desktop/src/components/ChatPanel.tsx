import { FormEvent, useEffect, useRef, useState } from "react";
import { cancelTimer, cancelTurn, clearScreenObservation, decideTool, finishPushToTalk, getScreenObservationPreview, importCharacterVrm, isTauri, openVoicePrivacySettings, requestVoicePermissions, setListening, startPushToTalk, stopPlayback, submitText, updatePetSettings, updateVoiceSettings } from "../lib/tauri";
import { formatTimerRemaining } from "../lib/timerFormat";
import { useAppStore } from "../stores/app";
import { ProviderSettingsDialog } from "./ProviderSettingsDialog";
import { MemoryPanel } from "./MemoryPanel";

export function ChatPanel() {
  const { snapshot, streamingText, booting, setProvider, setPet, setCharacter, setVoiceSettings } = useAppStore();
  const [draft, setDraft] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [submitError, setSubmitError] = useState("");
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [memoryOpen, setMemoryOpen] = useState(false);
  const [timersOpen, setTimersOpen] = useState(false);
  const [toolsOpen, setToolsOpen] = useState(false);
  const [petUpdating, setPetUpdating] = useState(false);
  const [petError, setPetError] = useState("");
  const [importingVrm, setImportingVrm] = useState(false);
  const [voiceBusy, setVoiceBusy] = useState(false);
  const [voiceError, setVoiceError] = useState("");
  const [toolBusy, setToolBusy] = useState(false);
  const [toolError, setToolError] = useState("");
  const [timerBusy, setTimerBusy] = useState(false);
  const [timerError, setTimerError] = useState("");
  const [screenPreviewUrl, setScreenPreviewUrl] = useState<string | null>(null);
  const [screenPreviewError, setScreenPreviewError] = useState("");
  const [screenPreviewBusy, setScreenPreviewBusy] = useState(false);
  const [clockMs, setClockMs] = useState(() => Date.now());
  const pttStart = useRef<Promise<void> | null>(null);
  const voicePermissionNotDetermined = snapshot.voice_permissions.microphone === "not_determined"
    || snapshot.voice_permissions.speech_recognition === "not_determined";
  const voicePermissionDenied = snapshot.voice_permissions.microphone === "denied"
    || snapshot.voice_permissions.speech_recognition === "denied";
  const voicePermissionRestricted = snapshot.voice_permissions.microphone === "restricted"
    || snapshot.voice_permissions.speech_recognition === "restricted";

  useEffect(() => {
    const observation = snapshot.screen_observation;
    setScreenPreviewError("");
    setScreenPreviewUrl(null);
    if (!observation || !isTauri()) return undefined;
    let cancelled = false;
    void getScreenObservationPreview(observation.id)
      .then((preview) => {
        if (!cancelled) setScreenPreviewUrl(preview.data_url);
      })
      .catch((reason) => {
        if (cancelled) return;
        if (typeof reason === "string") setScreenPreviewError(reason);
        else if (reason && typeof reason === "object" && "message" in reason && typeof reason.message === "string") setScreenPreviewError(reason.message);
        else setScreenPreviewError("读取本地屏幕预览失败。");
      });
    return () => {
      cancelled = true;
    };
  }, [snapshot.screen_observation?.id]);

  useEffect(() => {
    const interval = window.setInterval(() => setClockMs(Date.now()), 1_000);
    return () => window.clearInterval(interval);
  }, []);

  const beginPtt = () => {
    if (pttStart.current || !isTauri()) return;
    setVoiceError("");
    const request = startPushToTalk().catch((reason) => {
      if (typeof reason === "string") setVoiceError(reason);
      else if (reason && typeof reason === "object" && "message" in reason && typeof reason.message === "string") setVoiceError(reason.message);
      else setVoiceError("按键说话启动失败。");
      throw reason;
    });
    pttStart.current = request;
  };

  const endPtt = async () => {
    const started = pttStart.current;
    if (!started) return;
    pttStart.current = null;
    try {
      await started;
      await finishPushToTalk();
    } catch {
      // beginPtt 已显示启动错误；没有成功 capture 时无需再 finalize。
    }
  };

  const runVoiceAction = async (action: () => Promise<void>) => {
    if (voiceBusy || !isTauri()) return;
    setVoiceBusy(true);
    setVoiceError("");
    try {
      await action();
    } catch (reason) {
      if (typeof reason === "string") setVoiceError(reason);
      else if (reason && typeof reason === "object" && "message" in reason && typeof reason.message === "string") setVoiceError(reason.message);
      else setVoiceError("语音操作失败。");
    } finally {
      setVoiceBusy(false);
    }
  };

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
    setSubmitError("");
    try {
      await submitText(text);
    } catch (reason) {
      if (typeof reason === "string") setSubmitError(reason);
      else if (reason && typeof reason === "object" && "message" in reason && typeof reason.message === "string") setSubmitError(reason.message);
      else setSubmitError("发送消息失败。");
    } finally {
      setSubmitting(false);
    }
  };

  const rereadScreen = async () => {
    if (submitting || !isTauri()) return;
    setSubmitting(true);
    setSubmitError("");
    try {
      await submitText("请帮我看一下当前屏幕");
    } catch (reason) {
      if (typeof reason === "string") setSubmitError(reason);
      else if (reason && typeof reason === "object" && "message" in reason && typeof reason.message === "string") setSubmitError(reason.message);
      else setSubmitError("重新读取屏幕失败。");
    } finally {
      setSubmitting(false);
    }
  };

  const decidePendingTool = async (approved: boolean) => {
    const pending = snapshot.pending_tool_approval;
    if (!pending || toolBusy || !isTauri()) return;
    setToolBusy(true);
    setToolError("");
    try {
      await decideTool(pending.tool_call_id, approved);
    } catch (reason) {
      if (typeof reason === "string") setToolError(reason);
      else if (reason && typeof reason === "object" && "message" in reason && typeof reason.message === "string") setToolError(reason.message);
      else setToolError("处理工具请求失败。");
    } finally {
      setToolBusy(false);
    }
  };

  const cancelScheduledTimer = async (timerId: string) => {
    if (timerBusy || !isTauri()) return;
    setTimerBusy(true);
    setTimerError("");
    try {
      await cancelTimer(timerId);
    } catch (reason) {
      if (typeof reason === "string") setTimerError(reason);
      else if (reason && typeof reason === "object" && "message" in reason && typeof reason.message === "string") setTimerError(reason.message);
      else setTimerError("取消提醒失败。");
    } finally {
      setTimerBusy(false);
    }
  };

  const clearScreenPreview = async () => {
    if (screenPreviewBusy || !isTauri()) return;
    setScreenPreviewBusy(true);
    setScreenPreviewError("");
    try {
      await clearScreenObservation();
    } catch (reason) {
      if (typeof reason === "string") setScreenPreviewError(reason);
      else if (reason && typeof reason === "object" && "message" in reason && typeof reason.message === "string") setScreenPreviewError(reason.message);
      else setScreenPreviewError("清除本地屏幕预览失败。");
    } finally {
      setScreenPreviewBusy(false);
    }
  };

  const formatTimerDue = (timestamp: number) => new Intl.DateTimeFormat("zh-CN", {
    month: "numeric",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(timestamp));

  const formatTimerHistoryTime = (timestamp: number | null) => timestamp === null
    ? "时间未知"
    : new Intl.DateTimeFormat("zh-CN", {
      month: "numeric",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    }).format(new Date(timestamp));

  const formatTimerHistoryStatus = (status: string) => status === "fired" ? "已触发" : "已取消";

  const formatToolName = (name: string) => ({
    "system.open_url": "打开网页",
    "timer.create": "创建提醒",
    "timer.cancel": "取消提醒",
    "memory.save": "保存记忆",
    "memory.delete": "删除记忆",
    "screen.observe": "读取屏幕",
  }[name] ?? name);

  const formatToolStatus = (status: string) => ({
    pending: "待处理",
    approved: "已授权",
    rejected: "已拒绝",
    succeeded: "已完成",
    failed: "失败",
    cancelled: "已取消",
  }[status] ?? status);

  const formatToolOrigin = (origin: string) => origin === "model_suggested" ? "模型提议" : origin === "user_explicit" ? "用户请求" : "系统任务";

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
          <button className="header-text-button" type="button" onClick={() => setMemoryOpen(true)}>记忆</button>
          <button className="header-text-button" type="button" onClick={() => setTimersOpen((open) => !open)}>提醒{snapshot.timers.length ? ` ${snapshot.timers.length}` : ""}</button>
          <button className="header-text-button" type="button" onClick={() => setToolsOpen((open) => !open)}>工具{snapshot.tool_history.length ? ` ${snapshot.tool_history.length}` : ""}</button>
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

      <nav className="voice-controls" aria-label="语音控制">
        <span className={`voice-dot state-${snapshot.input_state}`} />
        <span>{snapshot.audio_host_state === "ready" ? snapshot.input_state : `AudioHost: ${snapshot.audio_host_state}`}</span>
        {snapshot.voice_input_route !== "unknown" && (
          <span className={`voice-route ${snapshot.voice_input_route}`}>
            {snapshot.voice_input_route === "on_device" ? "本机识别" : "系统语音服务"}
          </span>
        )}
        {voicePermissionNotDetermined && (
          <button type="button" disabled={voiceBusy || !isTauri()} onClick={() => runVoiceAction(requestVoicePermissions)}>授权语音</button>
        )}
        {voicePermissionDenied && (
          <>
            <button type="button" disabled={voiceBusy || !isTauri()} onClick={() => runVoiceAction(openVoicePrivacySettings)}>打开系统设置</button>
            <button type="button" disabled={voiceBusy || !isTauri()} onClick={() => runVoiceAction(requestVoicePermissions)}>刷新权限</button>
          </>
        )}
        {voicePermissionRestricted && <span className="voice-error">语音权限受系统策略限制</span>}
        <button
          type="button"
          disabled={voiceBusy || !isTauri() || snapshot.voice_permissions.microphone !== "authorized" || snapshot.voice_permissions.speech_recognition !== "authorized"}
          onClick={() => runVoiceAction(() => setListening(snapshot.listening_mode !== "armed"))}
        >
          {snapshot.listening_mode === "armed" ? "关闭持续监听" : "开启持续监听"}
        </button>
        <button
          type="button"
          className={snapshot.input_state === "capturing" || snapshot.input_state === "recognizing" ? "active" : ""}
          disabled={voiceBusy || !isTauri() || snapshot.voice_permissions.microphone !== "authorized" || snapshot.voice_permissions.speech_recognition !== "authorized"}
          onPointerDown={(event) => {
            event.currentTarget.setPointerCapture(event.pointerId);
            beginPtt();
          }}
          onPointerUp={() => void endPtt()}
          onPointerCancel={() => void endPtt()}
        >按住说话</button>
        <label>
          <input
            type="checkbox"
            checked={snapshot.voice_settings.tts_enabled}
            disabled={voiceBusy || !isTauri()}
            onChange={(event) => {
              const enabled = event.target.checked;
              void runVoiceAction(async () => setVoiceSettings(await updateVoiceSettings(snapshot.voice_settings, enabled)));
            }}
          />
          朗读回复
        </label>
        {snapshot.playback_state !== "idle" && <button type="button" onClick={() => runVoiceAction(stopPlayback)}>停止朗读</button>}
        <span className="voice-shortcut">监听：⌘/Ctrl + Shift + L · PTT：⌘/Ctrl + Shift + M</span>
        {voiceError && <span className="voice-error">{voiceError}</span>}
      </nav>

      <div className={`asr-caption ${snapshot.asr_partial ? "" : "empty"}`}>{snapshot.asr_partial}</div>

      {snapshot.pending_tool_approval && (
        <section className="tool-approval" aria-label="工具确认">
          <div className="tool-approval-heading">
            <div>
              <p className="eyebrow">ACTION REQUEST</p>
              <h2>{snapshot.pending_tool_approval.display_name}</h2>
            </div>
            <span className="tool-risk-badge">需要确认</span>
          </div>
          <p className="tool-approval-summary">{snapshot.pending_tool_approval.summary}</p>
          {snapshot.pending_tool_approval.url && <code className="tool-url">{snapshot.pending_tool_approval.url}</code>}
          {snapshot.pending_tool_approval.host && <p className="tool-host">目标域名：{snapshot.pending_tool_approval.host}</p>}
          {snapshot.pending_tool_approval.title && <p className="tool-host">{snapshot.pending_tool_approval.tool_name === "memory.save" || snapshot.pending_tool_approval.tool_name === "memory.delete" ? "记忆内容" : "提醒内容"}：{snapshot.pending_tool_approval.title}</p>}
          {snapshot.pending_tool_approval.due_at_ms && <p className="tool-host">提醒时间：{formatTimerDue(snapshot.pending_tool_approval.due_at_ms)}</p>}
          <div className="tool-approval-actions">
            <button type="button" className="secondary" disabled={toolBusy} onClick={() => void decidePendingTool(false)}>取消</button>
            <button type="button" disabled={toolBusy} onClick={() => void decidePendingTool(true)}>{toolBusy ? "处理中…" : snapshot.pending_tool_approval.tool_name === "timer.create" ? "设置提醒" : snapshot.pending_tool_approval.tool_name === "timer.cancel" ? "取消提醒" : snapshot.pending_tool_approval.tool_name === "memory.save" ? "保存记忆" : snapshot.pending_tool_approval.tool_name === "memory.delete" ? "删除记忆" : snapshot.pending_tool_approval.tool_name === "screen.observe" ? "读取屏幕" : "允许打开"}</button>
          </div>
          {toolError && <p className="settings-error">{toolError}</p>}
        </section>
      )}

      {timersOpen && (
        <section className="timer-panel" aria-label="提醒管理">
          <div className="timer-panel-heading">
            <div>
              <p className="eyebrow">LOCAL TIMERS</p>
              <h2>提醒管理</h2>
            </div>
            <button className="icon-button" type="button" onClick={() => setTimersOpen(false)} aria-label="关闭提醒列表">×</button>
          </div>
          {snapshot.timers.length > 0 && (
            <div className="timer-section">
              <p className="timer-section-label">待处理</p>
              {snapshot.timers.map((timer) => (
                <article className="timer-item" key={timer.id}>
                  <div>
                    <p>{timer.title}</p>
                    <span>{formatTimerDue(timer.due_at_ms)} · {formatTimerRemaining(timer.due_at_ms, clockMs)} · {timer.timezone}</span>
                  </div>
                  <button type="button" disabled={timerBusy || !isTauri()} onClick={() => void cancelScheduledTimer(timer.id)}>取消</button>
                </article>
              ))}
            </div>
          )}
          {snapshot.timer_history.length > 0 && (
            <div className="timer-section timer-history">
              <p className="timer-section-label">最近记录</p>
              {snapshot.timer_history.map((history) => (
                <article className="timer-item timer-history-item" key={history.id}>
                  <div>
                    <p>{history.title}</p>
                    <span>计划 {formatTimerDue(history.due_at_ms)} · {formatTimerHistoryTime(history.completed_at_ms)}</span>
                  </div>
                  <span className={`timer-history-status ${history.status === "fired" ? "fired" : "cancelled"}`}>
                    {formatTimerHistoryStatus(history.status)}
                  </span>
                </article>
              ))}
            </div>
          )}
          {snapshot.timers.length === 0 && snapshot.timer_history.length === 0 && <p className="timer-empty">还没有提醒记录。</p>}
          {timerError && <p className="settings-error">{timerError}</p>}
        </section>
      )}

      {toolsOpen && (
        <section className="tool-history-panel" aria-label="工具记录">
          <div className="tool-history-heading">
            <div>
              <p className="eyebrow">LOCAL AUDIT</p>
              <h2>工具记录</h2>
            </div>
            <button className="icon-button" type="button" onClick={() => setToolsOpen(false)} aria-label="关闭工具记录">×</button>
          </div>
          <p className="tool-history-note">这里只显示本地工具的名称、来源和结果摘要，不展示原始参数。</p>
          {snapshot.tool_history.length > 0 ? snapshot.tool_history.map((run) => (
            <article className="tool-history-item" key={run.id}>
              <div className="tool-history-item-heading">
                <div>
                  <p>{formatToolName(run.tool_name)}</p>
                  <span>{formatToolOrigin(run.origin)} · {formatTimerHistoryTime(run.created_at_ms)}</span>
                </div>
                <span className={`tool-history-status status-${run.status}`}>{formatToolStatus(run.status)}</span>
              </div>
              {run.result_summary && <div className="tool-history-result">{run.result_summary}</div>}
            </article>
          )) : <p className="timer-empty">还没有工具调用记录。</p>}
        </section>
      )}

      {snapshot.screen_observation && (
        <section className="screen-preview" aria-label="本地屏幕预览">
          <div className="screen-preview-heading">
            <div>
              <p className="eyebrow">LOCAL ONLY</p>
              <h2>本地屏幕预览</h2>
            </div>
            <div className="screen-preview-actions">
              <button className="header-text-button" type="button" disabled={submitting || !isTauri()} onClick={() => void rereadScreen()}>重新读取</button>
              <button className="icon-button" type="button" disabled={screenPreviewBusy} onClick={() => void clearScreenPreview()} aria-label="清除屏幕预览">×</button>
            </div>
          </div>
          {screenPreviewUrl ? <img className="screen-preview-image" src={screenPreviewUrl} alt="当前屏幕的本地临时预览" /> : <p className="timer-empty">正在准备本地预览…</p>}
          <p className="screen-preview-note">截图只保存在当前应用进程内存中，未发送给聊天或视觉模型。{formatTimerRemaining(snapshot.screen_observation.expires_at_ms, clockMs)}后自动清除；也可以手动清除。</p>
          {screenPreviewError && <p className="settings-error">{screenPreviewError}</p>}
        </section>
      )}

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
      {submitError && <div className="error-banner">{submitError}</div>}

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
      {memoryOpen && <MemoryPanel onClose={() => setMemoryOpen(false)} />}
    </main>
  );
}
