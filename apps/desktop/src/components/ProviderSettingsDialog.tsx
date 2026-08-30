import { FormEvent, useState } from "react";
import type { ProviderSettings } from "../contracts";
import { testProvider, updateProvider } from "../lib/tauri";

interface ProviderSettingsDialogProps {
  initial: ProviderSettings;
  onClose: () => void;
  onSaved: (settings: ProviderSettings) => void;
}

export function ProviderSettingsDialog({ initial, onClose, onSaved }: ProviderSettingsDialogProps) {
  const [provider, setProvider] = useState(initial.provider);
  const [endpoint, setEndpoint] = useState(initial.endpoint);
  const [model, setModel] = useState(initial.model);
  const [apiKey, setApiKey] = useState("");
  const [clearApiKey, setClearApiKey] = useState(false);
  const [allowMemoryContext, setAllowMemoryContext] = useState(initial.allow_memory_context);
  const [visionEnabled, setVisionEnabled] = useState(initial.vision.enabled);
  const [visionEndpoint, setVisionEndpoint] = useState(initial.vision.endpoint);
  const [visionModel, setVisionModel] = useState(initial.vision.model);
  const [visionApiKey, setVisionApiKey] = useState("");
  const [clearVisionApiKey, setClearVisionApiKey] = useState(false);
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState("");
  const [error, setError] = useState("");

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    setSaving(true);
    setError("");
    try {
      const updated = await updateProvider({
        provider,
        endpoint: endpoint.trim(),
        model: model.trim(),
        api_key: apiKey.trim() || undefined,
        clear_api_key: clearApiKey,
        allow_memory_context: allowMemoryContext,
        vision: {
          enabled: visionEnabled,
          endpoint: visionEndpoint.trim(),
          model: visionModel.trim(),
          api_key: visionApiKey.trim() || undefined,
          clear_api_key: clearVisionApiKey,
        },
        expected_revision: initial.revision,
      });
      onSaved(updated);
      onClose();
    } catch (reason) {
      if (typeof reason === "string") {
        setError(reason);
      } else if (reason && typeof reason === "object" && "message" in reason && typeof reason.message === "string") {
        setError(reason.message);
      } else {
        setError("保存 Provider 设置失败。");
      }
    } finally {
      setSaving(false);
    }
  };

  const runProviderTest = async () => {
    if (testing || saving) return;
    setTesting(true);
    setError("");
    setTestResult("");
    try {
      setTestResult(await testProvider());
    } catch (reason) {
      if (typeof reason === "string") setError(reason);
      else if (reason && typeof reason === "object" && "message" in reason && typeof reason.message === "string") setError(reason.message);
      else setError("Provider 连接测试失败。");
    } finally {
      setTesting(false);
    }
  };

  return (
    <div className="dialog-backdrop" role="presentation" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <form className="settings-dialog" onSubmit={submit} aria-label="LLM Provider 设置">
        <header>
          <div>
            <p className="eyebrow">LLM PROVIDER</p>
            <h2>模型连接</h2>
          </div>
          <button type="button" className="icon-button" onClick={onClose} aria-label="关闭设置">×</button>
        </header>

        <label>
          Provider
          <select value={provider} onChange={(event) => setProvider(event.target.value as ProviderSettings["provider"])}>
            <option value="mock">Mock（本地测试）</option>
            <option value="openai_compatible">OpenAI-compatible</option>
          </select>
        </label>

        {provider === "openai_compatible" && (
          <>
            <label>
              Endpoint
              <input value={endpoint} onChange={(event) => setEndpoint(event.target.value)} placeholder="https://api.openai.com/v1" />
            </label>
            <label>
              Model
              <input value={model} onChange={(event) => setModel(event.target.value)} placeholder="模型名称" />
            </label>
            <label>
              API key
              <input
                type="password"
                value={apiKey}
                onChange={(event) => setApiKey(event.target.value)}
                placeholder={initial.has_api_key ? "已由本地 .env 或 Keychain 提供；留空则不修改" : "可选；Ollama/LM Studio 通常留空"}
                autoComplete="off"
                disabled={clearApiKey}
              />
            </label>
            <label className="check-row">
              <input type="checkbox" checked={clearApiKey} onChange={(event) => setClearApiKey(event.target.checked)} />
              删除当前 Keychain API key
            </label>
            <p className="settings-hint">API key 可选；本地 .env 中的 DESKTOP_AI_LLM_API_KEY 优先，表单新填的 Key 保存到 macOS Keychain。Ollama/LM Studio 等无鉴权服务可以留空。</p>
            <label className="check-row">
              <input type="checkbox" checked={allowMemoryContext} onChange={(event) => setAllowMemoryContext(event.target.checked)} />
              允许把本轮匹配到的已保存记忆发送给当前 LLM Provider
            </label>
            <p className="settings-hint">默认关闭。开启后，相关记忆正文会随模型请求发送到上方配置的 endpoint；未匹配的记忆不会发送。</p>
          </>
        )}

        <section className="settings-section" aria-label="视觉 Provider 配置">
          <p className="eyebrow">VISION PROVIDER</p>
          <h3>屏幕理解配置</h3>
          <p className="settings-hint">这里只保存独立的视觉模型连接信息，不会自动采集或上传屏幕。真正启用屏幕观察前仍会单独确认数据出站。</p>
          <label className="check-row">
            <input type="checkbox" checked={visionEnabled} onChange={(event) => setVisionEnabled(event.target.checked)} />
            启用视觉 Provider 配置
          </label>
          {visionEnabled && (
            <>
              <label>
                Vision Endpoint
                <input value={visionEndpoint} onChange={(event) => setVisionEndpoint(event.target.value)} placeholder="https://api.openai.com/v1" />
              </label>
              <label>
                Vision Model
                <input value={visionModel} onChange={(event) => setVisionModel(event.target.value)} placeholder="支持图片输入的模型名称" />
              </label>
              <label>
                Vision API key
                <input
                  type="password"
                  value={visionApiKey}
                  onChange={(event) => setVisionApiKey(event.target.value)}
                  placeholder={initial.vision.has_api_key ? "已保存在独立 Keychain 账户；留空则不修改" : "可选；本地兼容服务通常留空"}
                  autoComplete="off"
                  disabled={clearVisionApiKey}
                />
              </label>
              <label className="check-row">
                <input type="checkbox" checked={clearVisionApiKey} onChange={(event) => setClearVisionApiKey(event.target.checked)} />
                删除当前 Vision Keychain API key
              </label>
            </>
          )}
        </section>

        <p className="settings-hint">连接测试只发送固定测试文本，不携带聊天记录、记忆或屏幕图片；如果刚修改了表单，请先保存后再测试。</p>
        {testResult && <p className="settings-success" role="status">连接测试成功：{testResult}</p>}
        {error && <p className="settings-error">{error}</p>}
        <footer>
          <button type="button" className="secondary" disabled={saving || testing} onClick={() => void runProviderTest()}>{testing ? "测试中…" : "测试已保存配置"}</button>
          <button type="button" className="secondary" onClick={onClose}>取消</button>
          <button type="submit" disabled={saving || (provider === "openai_compatible" && (!endpoint.trim() || !model.trim())) || (visionEnabled && (!visionEndpoint.trim() || !visionModel.trim()))}>
            {saving ? "保存中…" : "保存"}
          </button>
        </footer>
      </form>
    </div>
  );
}
