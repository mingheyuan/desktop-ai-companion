use std::pin::Pin;

use async_trait::async_trait;
use futures_util::{Stream, StreamExt};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::contracts::{AppError, ChatMessage, MessageRole, ProviderSettings};

pub type LlmStream = Pin<Box<dyn Stream<Item = Result<LlmEvent, AppError>> + Send>>;

#[derive(Debug, Clone)]
pub enum LlmEvent {
    TextDelta(String),
    Completed,
}

#[derive(Debug, Clone)]
pub struct LlmRequest {
    pub user_text: String,
    pub history: Vec<ChatMessage>,
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn start(
        &self,
        request: LlmRequest,
        cancel: CancellationToken,
    ) -> Result<LlmStream, AppError>;
}

pub fn create_provider(settings: &ProviderSettings) -> Result<Box<dyn LlmProvider>, AppError> {
    match settings.provider.as_str() {
        "mock" => Ok(Box::new(MockProvider)),
        "openai_compatible" => {
            let api_key = std::env::var("DESKTOP_AI_LLM_API_KEY").map_err(|_| {
                AppError::new(
                    "authentication_failed",
                    "尚未配置 DESKTOP_AI_LLM_API_KEY。",
                    false,
                )
            })?;
            Ok(Box::new(OpenAiCompatibleProvider {
                client: reqwest::Client::new(),
                endpoint: settings.endpoint.clone(),
                model: settings.model.clone(),
                api_key,
            }))
        }
        other => Err(AppError::new(
            "invalid_request",
            format!("未知 LLM Provider：{other}"),
            false,
        )),
    }
}

struct MockProvider;

#[async_trait]
impl LlmProvider for MockProvider {
    async fn start(
        &self,
        request: LlmRequest,
        cancel: CancellationToken,
    ) -> Result<LlmStream, AppError> {
        let reply = format!(
            "任务确认，老师！爱丽丝收到了“{}”。文字对话核心已经正常运转，接下来可以继续解锁语音与记忆模块。",
            request.user_text
        );
        Ok(Box::pin(async_stream::stream! {
            for chunk in split_for_streaming(&reply) {
                tokio::select! {
                    _ = cancel.cancelled() => {
                        yield Err(AppError::new("cancelled", "对话已取消。", false));
                        return;
                    }
                    _ = tokio::time::sleep(std::time::Duration::from_millis(45)) => {
                        yield Ok(LlmEvent::TextDelta(chunk));
                    }
                }
            }
            yield Ok(LlmEvent::Completed);
        }))
    }
}

struct OpenAiCompatibleProvider {
    client: reqwest::Client,
    endpoint: String,
    model: String,
    api_key: String,
}

#[async_trait]
impl LlmProvider for OpenAiCompatibleProvider {
    async fn start(
        &self,
        request: LlmRequest,
        cancel: CancellationToken,
    ) -> Result<LlmStream, AppError> {
        let url = if self
            .endpoint
            .trim_end_matches('/')
            .ends_with("chat/completions")
        {
            self.endpoint.clone()
        } else {
            format!("{}/chat/completions", self.endpoint.trim_end_matches('/'))
        };
        let mut messages = vec![json!({
            "role": "system",
            "content": "你是桌面伙伴爱丽丝。认真、直接、友善，称呼用户为老师。不要声称拥有尚未实现的系统能力。"
        })];
        messages.extend(request.history.into_iter().filter(|message| !message.canonical_text.is_empty()).map(|message| {
            json!({
                "role": if message.role == MessageRole::User { "user" } else { "assistant" },
                "content": message.canonical_text,
            })
        }));
        messages.push(json!({"role": "user", "content": request.user_text}));

        let response = self
            .client
            .post(url)
            .bearer_auth(&self.api_key)
            .json(&json!({"model": self.model, "messages": messages, "stream": true}))
            .send()
            .await
            .map_err(|error| {
                AppError::new(
                    "provider_unavailable",
                    format!("LLM 连接失败：{error}"),
                    true,
                )
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let retryable = status.as_u16() == 429 || status.is_server_error();
            return Err(AppError::new(
                if status.as_u16() == 401 {
                    "authentication_failed"
                } else {
                    "provider_unavailable"
                },
                format!("LLM 返回 HTTP {status}"),
                retryable,
            ));
        }

        let mut bytes = response.bytes_stream();
        Ok(Box::pin(async_stream::stream! {
            let mut buffer = Vec::<u8>::new();
            loop {
                let next = tokio::select! {
                    _ = cancel.cancelled() => {
                        yield Err(AppError::new("cancelled", "对话已取消。", false));
                        return;
                    }
                    value = bytes.next() => value,
                };
                let Some(next) = next else { break };
                let chunk = match next {
                    Ok(chunk) => chunk,
                    Err(error) => {
                        yield Err(AppError::new("provider_unavailable", format!("LLM 流中断：{error}"), true));
                        return;
                    }
                };
                buffer.extend_from_slice(&chunk);
                while let Some(end) = find_sse_event(&buffer) {
                    let event = buffer.drain(..end).collect::<Vec<_>>();
                    let drain_separator = if buffer.starts_with(b"\r\n\r\n") { 4 } else { 2 };
                    buffer.drain(..drain_separator.min(buffer.len()));
                    let text = String::from_utf8_lossy(&event);
                    for line in text.lines().filter_map(|line| line.strip_prefix("data:")) {
                        let data = line.trim();
                        if data == "[DONE]" {
                            yield Ok(LlmEvent::Completed);
                            return;
                        }
                        match serde_json::from_str::<Value>(data) {
                            Ok(value) => {
                                if let Some(delta) = value.pointer("/choices/0/delta/content").and_then(Value::as_str) {
                                    if !delta.is_empty() {
                                        yield Ok(LlmEvent::TextDelta(delta.to_owned()));
                                    }
                                }
                            }
                            Err(error) => {
                                yield Err(AppError::new("protocol_mismatch", format!("无法解析 LLM SSE：{error}"), false));
                                return;
                            }
                        }
                    }
                }
            }
            yield Ok(LlmEvent::Completed);
        }))
    }
}

fn find_sse_event(buffer: &[u8]) -> Option<usize> {
    buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .or_else(|| buffer.windows(2).position(|window| window == b"\n\n"))
}

fn split_for_streaming(text: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    for character in text.chars() {
        current.push(character);
        if current.chars().count() >= 6 || "，。！？；".contains(character) {
            chunks.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_both_sse_separators() {
        assert_eq!(find_sse_event(b"data: x\n\nrest"), Some(7));
        assert_eq!(find_sse_event(b"data: x\r\n\r\nrest"), Some(7));
    }

    #[tokio::test]
    async fn mock_provider_streams_and_completes() {
        let provider = MockProvider;
        let mut stream = provider
            .start(
                LlmRequest {
                    user_text: "测试".into(),
                    history: vec![],
                },
                CancellationToken::new(),
            )
            .await
            .unwrap();
        let mut text = String::new();
        let mut completed = false;
        while let Some(event) = stream.next().await {
            match event.unwrap() {
                LlmEvent::TextDelta(delta) => text.push_str(&delta),
                LlmEvent::Completed => completed = true,
            }
        }
        assert!(completed);
        assert!(text.contains("测试"));
    }
}
