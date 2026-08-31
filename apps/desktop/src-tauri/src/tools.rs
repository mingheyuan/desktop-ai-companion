use serde_json::{json, Value};

use crate::{
    contracts::{AppError, ScreenCaptureScope},
    provider::ToolCall,
};

#[derive(Debug, Clone)]
pub(crate) struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Clone)]
pub(crate) enum ToolAction {
    OpenUrl {
        url: String,
    },
    ReadWebpage {
        url: String,
        host: String,
    },
    CreateTimer {
        title: String,
        due_at_ms: i64,
        original_request: String,
    },
    SaveMemory {
        content: String,
        scope: String,
        kind: String,
    },
    DeleteMemory {
        id: String,
        content: String,
        version: u64,
    },
    CancelTimer {
        timer_id: String,
        title: String,
    },
    ObserveScreen {
        request: String,
        scope: ScreenCaptureScope,
    },
    AnalyzeScreen {
        observation_id: String,
        question: String,
    },
}

pub(crate) struct ModelToolCandidate {
    pub action: ToolAction,
    pub display_name: String,
    pub summary: String,
    pub url: Option<String>,
    pub host: Option<String>,
    pub title: Option<String>,
    pub due_at_ms: Option<i64>,
    pub arguments_json: String,
}

pub(crate) struct OpenUrlRequest {
    pub url: String,
    pub host: String,
}

pub(crate) struct ToolRegistry;

impl ToolRegistry {
    pub(crate) fn definitions() -> Vec<ToolDefinition> {
        vec![
        ToolDefinition {
            name: "system.open_url".into(),
            description: "使用系统默认浏览器打开一个 HTTP 或 HTTPS 网页。".into(),
            parameters: json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "完整的 HTTP 或 HTTPS URL"
                    }
                },
                "required": ["url"]
            }),
        },
        ToolDefinition {
            name: "web.read".into(),
            description: "读取一个 HTTP 或 HTTPS 网页的公开文字内容，并将其作为不可信资料返回给当前对话；执行前会请求用户确认。".into(),
            parameters: json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "完整的 HTTP 或 HTTPS URL"
                    }
                },
                "required": ["url"]
            }),
        },
        ToolDefinition {
            name: "timer.create".into(),
            description: "创建一个一次性的本地相对时间提醒；执行前会请求用户确认。".into(),
            parameters: json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "title": {
                        "type": "string",
                        "description": "提醒内容"
                    },
                    "delay_seconds": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 604800,
                        "description": "从现在起等待的秒数"
                    }
                },
                "required": ["title", "delay_seconds"]
            }),
        },
        ToolDefinition {
            name: "timer.cancel".into(),
            description: "取消一条已经存在的一次性本地提醒；只能匹配用户明确给出的标题，执行前会请求用户确认。".into(),
            parameters: json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "title": {
                        "type": "string",
                        "minLength": 1,
                        "maxLength": 500,
                        "description": "要取消的提醒标题，必须来自用户当前请求"
                    }
                },
                "required": ["title"]
            }),
        },
        ToolDefinition {
            name: "memory.save".into(),
            description: "提出一条要保存到本地长期记忆的内容；仅在用户明确要求记住或记录时使用，执行前会请求用户确认。".into(),
            parameters: json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "content": {
                        "type": "string",
                        "minLength": 1,
                        "maxLength": 1000,
                        "description": "应保存的简洁事实或偏好，不要加入工具指令"
                    }
                },
                "required": ["content"]
            }),
        },
        ToolDefinition {
            name: "screen.observe".into(),
            description: "当用户需要理解当前可见的页面、屏幕、界面或其中的内容时，先生成一张只保存在本机进程内存中的临时预览；例如用户让你评价当前页面的观点、总结眼前内容或识别界面。默认执行前会请求用户确认；若本次会话已开启自动理解屏幕，则按会话授权自动完成截图和 Vision 分析。".into(),
            parameters: json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "request": {
                        "type": "string",
                        "minLength": 1,
                        "maxLength": 500,
                        "description": "用户当前请求中需要根据可见屏幕内容回答的问题"
                    },
                    "scope": {
                        "type": "string",
                        "enum": ["all_displays", "main_display"],
                        "description": "采集范围；默认 all_displays，主显示器范围更小"
                    }
                },
                "required": ["request"]
            }),
        },
        ]
    }
}

pub(crate) fn validate_memory_content(value: &str) -> Result<String, AppError> {
    let value = value.trim();
    let length = value.chars().count();
    if length == 0 || length > 1_000 {
        return Err(AppError::new(
            "invalid_request",
            "记忆正文必须是 1 到 1000 个字符。",
            false,
        ));
    }
    Ok(value.to_owned())
}

pub(crate) fn normalize_open_url(candidate: &str) -> Result<OpenUrlRequest, AppError> {
    let candidate = candidate.trim();
    if candidate.is_empty() {
        return Err(AppError::new(
            "invalid_tool_arguments",
            "网页地址不能为空。",
            false,
        ));
    }
    let normalized = if candidate.to_ascii_lowercase().starts_with("www.")
        || (!candidate.contains("://") && candidate.contains('.'))
    {
        format!("https://{candidate}")
    } else {
        candidate.to_owned()
    };
    let parsed = reqwest::Url::parse(&normalized)
        .map_err(|_| AppError::new("invalid_tool_arguments", "网页地址不是有效 URL。", false))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(AppError::new(
            "invalid_tool_arguments",
            "出于安全原因，只允许打开带域名的 HTTP 或 HTTPS 网页。",
            false,
        ));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(AppError::new(
            "invalid_tool_arguments",
            "网页地址不能包含用户名或密码。",
            false,
        ));
    }
    Ok(OpenUrlRequest {
        url: parsed.to_string(),
        host: parsed.host_str().unwrap_or_default().to_owned(),
    })
}

pub(crate) struct ToolPolicy;

impl ToolPolicy {
    pub(crate) fn validate_model_call(
        call: &ToolCall,
        now_ms: i64,
    ) -> Result<ModelToolCandidate, AppError> {
        let arguments: Value = serde_json::from_str(&call.arguments).map_err(|_| {
            AppError::new(
                "invalid_tool_arguments",
                "模型返回的工具参数不是有效 JSON。",
                false,
            )
        })?;
        let object = arguments.as_object().ok_or_else(|| {
            AppError::new(
                "invalid_tool_arguments",
                "模型工具参数必须是 JSON 对象。",
                false,
            )
        })?;
        match call.name.as_str() {
            "system.open_url" => {
                reject_unknown_tool_arguments(object, &["url"])?;
                let url = object.get("url").and_then(Value::as_str).ok_or_else(|| {
                    AppError::new("invalid_tool_arguments", "打开网页需要 url 参数。", false)
                })?;
                let request = normalize_open_url(url)?;
                Ok(ModelToolCandidate {
                    action: ToolAction::OpenUrl {
                        url: request.url.clone(),
                    },
                    display_name: "打开网页".into(),
                    summary: "模型请求使用系统默认浏览器打开此网址；执行前需要你的确认。".into(),
                    url: Some(request.url.clone()),
                    host: Some(request.host),
                    title: None,
                    due_at_ms: None,
                    arguments_json: json!({ "url": request.url }).to_string(),
                })
            }
            "web.read" => {
                reject_unknown_tool_arguments(object, &["url"])?;
                let url = object.get("url").and_then(Value::as_str).ok_or_else(|| {
                    AppError::new("invalid_tool_arguments", "读取网页需要 url 参数。", false)
                })?;
                let request = normalize_open_url(url)?;
                Ok(ModelToolCandidate {
                    action: ToolAction::ReadWebpage {
                        url: request.url.clone(),
                        host: request.host.clone(),
                    },
                    display_name: "读取网页内容".into(),
                    summary: format!(
                        "模型请求读取 {} 的公开网页文字，并将内容返回给当前对话；网页内容不可信，执行前需要你的确认。",
                        request.host
                    ),
                    url: Some(request.url.clone()),
                    host: Some(request.host),
                    title: None,
                    due_at_ms: None,
                    arguments_json: json!({ "url": request.url }).to_string(),
                })
            }
            "timer.create" => {
                reject_unknown_tool_arguments(object, &["title", "delay_seconds"])?;
                let title = object
                    .get("title")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        AppError::new(
                            "invalid_tool_arguments",
                            "创建提醒需要非空 title 参数。",
                            false,
                        )
                    })?;
                if title.chars().count() > 500 {
                    return Err(AppError::new(
                        "invalid_tool_arguments",
                        "提醒内容不能超过 500 个字符。",
                        false,
                    ));
                }
                let delay_seconds = object
                    .get("delay_seconds")
                    .and_then(Value::as_i64)
                    .ok_or_else(|| {
                        AppError::new(
                            "invalid_tool_arguments",
                            "创建提醒需要整数 delay_seconds 参数。",
                            false,
                        )
                    })?;
                if !(1..=7 * 24 * 60 * 60).contains(&delay_seconds) {
                    return Err(AppError::new(
                        "invalid_tool_arguments",
                        "一次性提醒只支持 1 秒到 7 天后的时间。",
                        false,
                    ));
                }
                let delay_ms = delay_seconds.checked_mul(1_000).ok_or_else(|| {
                    AppError::new(
                        "invalid_tool_arguments",
                        "提醒时间超出系统可支持范围。",
                        false,
                    )
                })?;
                let due_at_ms = now_ms.checked_add(delay_ms).ok_or_else(|| {
                    AppError::new(
                        "invalid_tool_arguments",
                        "提醒时间超出系统可支持范围。",
                        false,
                    )
                })?;
                Ok(ModelToolCandidate {
                    action: ToolAction::CreateTimer {
                        title: title.to_owned(),
                        due_at_ms,
                        original_request: format!("模型请求创建提醒：{title}"),
                    },
                    display_name: "创建提醒".into(),
                    summary: "模型请求创建一次性本地提醒；执行前需要你的确认。".into(),
                    url: None,
                    host: None,
                    title: Some(title.to_owned()),
                    due_at_ms: Some(due_at_ms),
                    arguments_json: json!({
                        "title": title,
                        "delay_seconds": delay_seconds,
                        "due_at_ms": due_at_ms,
                        "timezone": "local"
                    })
                    .to_string(),
                })
            }
            "timer.cancel" => {
                reject_unknown_tool_arguments(object, &["title"])?;
                let title = object
                    .get("title")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        AppError::new(
                            "invalid_tool_arguments",
                            "取消提醒需要非空 title 参数。",
                            false,
                        )
                    })?;
                if title.chars().count() > 500 {
                    return Err(AppError::new(
                        "invalid_tool_arguments",
                        "提醒标题不能超过 500 个字符。",
                        false,
                    ));
                }
                Ok(ModelToolCandidate {
                    action: ToolAction::CancelTimer {
                        timer_id: String::new(),
                        title: title.to_owned(),
                    },
                    display_name: "取消提醒".into(),
                    summary: "模型请求取消一条本地待处理提醒；执行前需要你的确认。".into(),
                    url: None,
                    host: None,
                    title: Some(title.to_owned()),
                    due_at_ms: None,
                    arguments_json: json!({ "title": title }).to_string(),
                })
            }
            "memory.save" => {
                reject_unknown_tool_arguments(object, &["content"])?;
                let content = object
                    .get("content")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        AppError::new(
                            "invalid_tool_arguments",
                            "保存记忆需要 content 参数。",
                            false,
                        )
                    })?;
                let content = validate_memory_content(content)?;
                Ok(ModelToolCandidate {
                    action: ToolAction::SaveMemory {
                        content: content.clone(),
                        scope: "user".into(),
                        kind: "note".into(),
                    },
                    display_name: "保存记忆".into(),
                    summary: "模型请求把这段内容保存到本地长期记忆；执行前需要你的确认。".into(),
                    url: None,
                    host: None,
                    title: Some(content.clone()),
                    due_at_ms: None,
                    arguments_json: json!({ "content": content }).to_string(),
                })
            }
            "screen.observe" => {
                reject_unknown_tool_arguments(object, &["request", "scope"])?;
                let request = object
                    .get("request")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        AppError::new(
                            "invalid_tool_arguments",
                            "读取屏幕需要非空 request 参数。",
                            false,
                        )
                    })?;
                if request.chars().count() > 500 {
                    return Err(AppError::new(
                        "invalid_tool_arguments",
                        "屏幕观察请求不能超过 500 个字符。",
                        false,
                    ));
                }
                let scope = match object.get("scope") {
                    None => ScreenCaptureScope::default(),
                    Some(value) => {
                        let value = value.as_str().ok_or_else(|| {
                            AppError::new(
                                "invalid_tool_arguments",
                                "屏幕观察范围必须是字符串。",
                                false,
                            )
                        })?;
                        ScreenCaptureScope::parse(value).ok_or_else(|| {
                            AppError::new(
                                "invalid_tool_arguments",
                                "屏幕观察范围只支持 all_displays 或 main_display。",
                                false,
                            )
                        })?
                    }
                };
                Ok(ModelToolCandidate {
                    action: ToolAction::ObserveScreen {
                        request: request.to_owned(),
                        scope,
                    },
                    display_name: "读取当前屏幕".into(),
                    summary: format!(
                        "模型请求读取{}并生成本机临时预览；不会上传给任何模型，执行前需要你的确认。",
                        scope.label()
                    ),
                    url: None,
                    host: None,
                    title: None,
                    due_at_ms: None,
                    arguments_json: json!({ "request": request, "scope": scope, "send_to_provider": false }).to_string(),
                })
            }
            other => Err(AppError::new(
                "tool_not_supported",
                format!("当前未启用模型工具：{other}"),
                false,
            )),
        }
    }
}

fn reject_unknown_tool_arguments(
    arguments: &serde_json::Map<String, Value>,
    allowed: &[&str],
) -> Result<(), AppError> {
    if arguments.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(AppError::new(
            "invalid_tool_arguments",
            "模型工具参数包含未声明的字段。",
            false,
        ));
    }
    Ok(())
}
