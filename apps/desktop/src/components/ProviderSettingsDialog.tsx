import { FormEvent, useState } from "react";
import type { ProviderSettings } from "../contracts";
import { updateProvider } from "../lib/tauri";

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
  const [saving, setSaving] = useState(false);
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
                placeholder={initial.has_api_key ? "已保存在 macOS Keychain；留空则不修改" : "保存到 macOS Keychain"}
                autoComplete="off"
                disabled={clearApiKey}
              />
            </label>
            <label className="check-row">
              <input type="checkbox" checked={clearApiKey} onChange={(event) => setClearApiKey(event.target.checked)} />
              删除当前 Keychain API key
            </label>
            <p className="settings-hint">API key 不进入 SQLite、前端持久存储或日志。环境变量只作为开发期回退。</p>
          </>
        )}

        {error && <p className="settings-error">{error}</p>}
        <footer>
          <button type="button" className="secondary" onClick={onClose}>取消</button>
          <button type="submit" disabled={saving || (provider === "openai_compatible" && (!endpoint.trim() || !model.trim()))}>
            {saving ? "保存中…" : "保存"}
          </button>
        </footer>
      </form>
    </div>
  );
}
