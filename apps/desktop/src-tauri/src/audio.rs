use std::{
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use rand::RngCore;
use serde_json::{json, Value};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::unix::OwnedWriteHalf,
    net::UnixListener,
    process::Command,
    sync::{mpsc, Mutex},
};
use uuid::Uuid;

use crate::contracts::AppError;

pub type AudioWriter = Arc<Mutex<OwnedWriteHalf>>;

pub struct AudioConnection {
    pub writer: AudioWriter,
    pub events: mpsc::UnboundedReceiver<Value>,
    pub connection_id: String,
}

pub async fn launch(bundled_app: Option<PathBuf>) -> Result<AudioConnection, AppError> {
    let id = Uuid::now_v7().simple().to_string();
    let socket_path = PathBuf::from(format!("/tmp/dac-{id}.sock"));
    let token_path = PathBuf::from(format!("/tmp/dac-{id}.token"));
    let launch_nonce = Uuid::now_v7().to_string();
    let token = create_token();
    write_token_file(&token_path, &token)?;

    let listener = UnixListener::bind(&socket_path).map_err(|error| {
        cleanup_paths(&socket_path, &token_path);
        AppError::new(
            "audio_host_failed",
            format!("无法创建 AudioHost socket：{error}"),
            true,
        )
    })?;
    let app_path = resolve_audio_host_app(bundled_app)?;
    let mut child = Command::new("/usr/bin/open")
        .arg("-n")
        .arg("-W")
        .arg("-a")
        .arg(&app_path)
        .arg("--args")
        .arg("--socket")
        .arg(&socket_path)
        .arg("--token-file")
        .arg(&token_path)
        .arg("--launch-nonce")
        .arg(&launch_nonce)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| {
            cleanup_paths(&socket_path, &token_path);
            AppError::new(
                "audio_host_failed",
                format!("无法启动 AudioHost.app：{error}"),
                true,
            )
        })?;

    let stream = tokio::select! {
        accepted = listener.accept() => accepted.map(|value| value.0).map_err(|error| {
            AppError::new("audio_host_failed", format!("AudioHost 连接失败：{error}"), true)
        })?,
        status = child.wait() => {
            cleanup_paths(&socket_path, &token_path);
            return Err(AppError::new(
                "audio_host_failed",
                format!("AudioHost 在握手前退出：{}", status.map(|value| value.to_string()).unwrap_or_else(|error| error.to_string())),
                true,
            ));
        }
        _ = tokio::time::sleep(Duration::from_secs(15)) => {
            let _ = child.kill().await;
            cleanup_paths(&socket_path, &token_path);
            return Err(AppError::new("audio_host_failed", "AudioHost 连接超时。", true));
        }
    };
    drop(listener);
    cleanup_paths(&socket_path, &token_path);

    let connection_id = launch_nonce.clone();
    let (reader, writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();
    let hello_line = tokio::time::timeout(Duration::from_secs(5), lines.next_line())
        .await
        .map_err(|_| AppError::new("audio_host_failed", "AudioHost 握手超时。", true))?
        .map_err(|error| {
            AppError::new(
                "audio_host_failed",
                format!("无法读取 AudioHost 握手：{error}"),
                true,
            )
        })?
        .ok_or_else(|| AppError::new("audio_host_failed", "AudioHost 未发送握手。", true))?;
    let hello: Value = serde_json::from_str(&hello_line).map_err(|error| {
        AppError::new(
            "protocol_mismatch",
            format!("AudioHost 握手不是有效 JSON：{error}"),
            false,
        )
    })?;
    let valid = hello.get("type").and_then(Value::as_str) == Some("host.hello")
        && hello.pointer("/protocol/major").and_then(Value::as_u64) == Some(1)
        && hello.get("launch_nonce").and_then(Value::as_str) == Some(launch_nonce.as_str())
        && hello.get("token").and_then(Value::as_str) == Some(token.as_str());
    if !valid {
        return Err(AppError::new(
            "protocol_mismatch",
            "AudioHost 握手 token、nonce 或协议版本不匹配。",
            false,
        ));
    }

    let writer = Arc::new(Mutex::new(writer));
    send(
        &writer,
        json!({"type": "host.ack", "protocol": {"major": 1, "minor": 0}}),
    )
    .await?;
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let event_connection_id = connection_id.clone();
    tauri::async_runtime::spawn(async move {
        while let Ok(Some(line)) = lines.next_line().await {
            if let Ok(mut event) = serde_json::from_str::<Value>(&line) {
                if let Some(object) = event.as_object_mut() {
                    object.insert(
                        "_connection_id".into(),
                        Value::String(event_connection_id.clone()),
                    );
                }
                if event_tx.send(event).is_err() {
                    break;
                }
            }
        }
        let _ = event_tx.send(json!({
            "type": "host.disconnected",
            "_connection_id": event_connection_id
        }));
        let _ = child.wait().await;
    });
    Ok(AudioConnection {
        writer,
        events: event_rx,
        connection_id,
    })
}

pub async fn send(writer: &AudioWriter, value: Value) -> Result<(), AppError> {
    let mut payload = serde_json::to_vec(&value).map_err(|error| {
        AppError::new(
            "internal",
            format!("无法编码 AudioHost 消息：{error}"),
            false,
        )
    })?;
    payload.push(b'\n');
    let mut writer = writer.lock().await;
    writer.write_all(&payload).await.map_err(|error| {
        AppError::new(
            "audio_host_failed",
            format!("无法写入 AudioHost：{error}"),
            true,
        )
    })?;
    writer.flush().await.map_err(|error| {
        AppError::new(
            "audio_host_failed",
            format!("无法刷新 AudioHost 消息：{error}"),
            true,
        )
    })
}

fn resolve_audio_host_app(bundled_app: Option<PathBuf>) -> Result<PathBuf, AppError> {
    let path = std::env::var_os("DESKTOP_AI_AUDIO_HOST_APP")
        .map(PathBuf::from)
        .or_else(|| bundled_app.filter(|path| path.join("Contents/MacOS/audio-host").is_file()))
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .nth(3)
                .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")))
                .join("target/AudioHost.app")
        });
    if !path.join("Contents/MacOS/audio-host").is_file() {
        return Err(AppError::new(
            "audio_host_missing",
            format!(
                "找不到 AudioHost.app。请先运行 scripts/build-audio-host.sh（期望位置：{}）。",
                path.display()
            ),
            false,
        ));
    }
    Ok(path)
}

fn create_token() -> String {
    let mut bytes = [0_u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn write_token_file(path: &Path, token: &str) -> Result<(), AppError> {
    use std::io::Write;

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|error| {
            AppError::new(
                "audio_host_failed",
                format!("无法创建 AudioHost token 文件：{error}"),
                true,
            )
        })?;
    file.write_all(token.as_bytes()).map_err(|error| {
        AppError::new(
            "audio_host_failed",
            format!("无法写入 AudioHost token 文件：{error}"),
            true,
        )
    })
}

fn cleanup_paths(socket_path: &Path, token_path: &Path) {
    let _ = std::fs::remove_file(socket_path);
    let _ = std::fs::remove_file(token_path);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handshake_tokens_have_256_bits() {
        let token = create_token();
        assert_eq!(token.len(), 64);
        assert!(token.chars().all(|value| value.is_ascii_hexdigit()));
    }

    #[tokio::test]
    #[ignore = "launches the signed macOS app bundle"]
    async fn launches_audio_host_and_completes_handshake() {
        assert_audio_host_handshake(None).await;
    }

    #[tokio::test]
    #[ignore = "requires pnpm package:macos --debug"]
    async fn launches_packaged_audio_host_and_completes_handshake() {
        let bundled_app = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .expect("repository root")
            .join(
                "target/debug/bundle/macos/Desktop AI Companion.app/Contents/Resources/AudioHost.app",
            );
        assert!(
            bundled_app.join("Contents/MacOS/audio-host").is_file(),
            "run pnpm package:macos --debug before this test"
        );
        assert_audio_host_handshake(Some(bundled_app)).await;
    }

    async fn assert_audio_host_handshake(bundled_app: Option<PathBuf>) {
        let mut connection = launch(bundled_app).await.expect("AudioHost should launch");
        let ready = tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(event) = connection.events.recv().await {
                if event.get("type").and_then(Value::as_str) == Some("host.ready") {
                    return true;
                }
            }
            false
        })
        .await
        .expect("AudioHost should become ready before timeout");
        assert!(ready);
        send(&connection.writer, json!({"type": "core.ping", "seq": 1}))
            .await
            .expect("AudioHost should accept heartbeat");
        let pong = tokio::time::timeout(Duration::from_secs(3), async {
            while let Some(event) = connection.events.recv().await {
                if event.get("type").and_then(Value::as_str) == Some("host.pong")
                    && event.get("seq").and_then(Value::as_u64) == Some(1)
                {
                    return true;
                }
            }
            false
        })
        .await
        .expect("AudioHost should respond to heartbeat");
        assert!(pong);
        send(&connection.writer, json!({"type": "shutdown"}))
            .await
            .expect("AudioHost should accept shutdown");
    }
}
