use crate::contracts::{AppError, TimerItem};
use tauri::AppHandle;

/// Keep the public notification boundary platform-neutral. macOS uses a user
/// LaunchAgent so a one-shot reminder can outlive the Tauri process; other
/// platforms keep the existing in-process scheduler as the only implementation.
pub async fn schedule_timer(app: &AppHandle, timer: &TimerItem) -> Result<(), AppError> {
    #[cfg(target_os = "macos")]
    {
        imp::schedule_timer(app, timer).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, timer);
        Ok(())
    }
}

/// Remove the persistent system job and its claim marker for a user-cancelled
/// reminder. The operation is intentionally idempotent.
pub async fn cancel_timer(app: &AppHandle, timer_id: &str) -> Result<(), AppError> {
    #[cfg(target_os = "macos")]
    {
        imp::cancel_timer(app, timer_id).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, timer_id);
        Ok(())
    }
}

/// Rebuild persistent jobs from SQLite on application startup.
pub async fn reconcile(app: &AppHandle, timers: &[TimerItem]) -> Result<(), AppError> {
    #[cfg(target_os = "macos")]
    {
        imp::reconcile(app, timers).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, timers);
        Ok(())
    }
}

/// Atomically claim the right to display a notification for a due timer.
///
/// The in-process scheduler and the LaunchAgent both use this marker. Only
/// one of them can create the directory, so an app-open and app-closed path
/// cannot normally display the same reminder twice.
pub async fn claim_notification(app: &AppHandle, timer_id: &str) -> Result<bool, AppError> {
    #[cfg(target_os = "macos")]
    {
        imp::claim_notification(app, timer_id).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, timer_id);
        Ok(true)
    }
}

/// Remove only the LaunchAgent files after a timer has fired. The claim
/// marker is deliberately retained until the SQLite row is no longer pending,
/// which closes the duplicate-notification race during app restart.
pub async fn cleanup_fired_job(app: &AppHandle, timer_id: &str) -> Result<(), AppError> {
    #[cfg(target_os = "macos")]
    {
        imp::cleanup_fired_job(app, timer_id).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, timer_id);
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::{
        collections::HashSet,
        io::ErrorKind,
        path::{Path, PathBuf},
        process::Stdio,
        time::{SystemTime, UNIX_EPOCH},
    };

    use tauri::{AppHandle, Manager};
    use tokio::{fs, process::Command};

    use crate::{contracts::AppError, contracts::TimerItem};

    const LABEL_PREFIX: &str = "com.mingyuan.desktop-ai-companion.timer.";

    struct Paths {
        launch_agents: PathBuf,
        jobs: PathBuf,
        scripts: PathBuf,
        claims: PathBuf,
    }

    struct LaunchAgentSpec<'a> {
        label: &'a str,
        script_path: &'a Path,
        claims_path: &'a Path,
        timer_id: &'a str,
        due_at_ms: i64,
        plist_path: &'a Path,
        apple_script_path: &'a Path,
        uid: &'a str,
    }

    impl Paths {
        fn for_app(app: &AppHandle) -> Result<Self, AppError> {
            let home = std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| {
                AppError::new("notification_unavailable", "无法定位当前用户目录。", true)
            })?;
            let app_data = app.path().app_data_dir().map_err(|error| {
                AppError::new(
                    "notification_unavailable",
                    format!("无法定位提醒数据目录：{error}"),
                    true,
                )
            })?;
            let root = app_data.join("timer-notifications");
            Ok(Self {
                launch_agents: home.join("Library/LaunchAgents"),
                jobs: root.join("jobs"),
                scripts: root.join("scripts"),
                claims: root.join("claims"),
            })
        }

        async fn ensure(&self) -> Result<(), AppError> {
            fs::create_dir_all(&self.launch_agents)
                .await
                .map_err(|error| {
                    AppError::new(
                        "notification_unavailable",
                        format!(
                            "无法创建系统提醒目录 {}：{error}",
                            self.launch_agents.display()
                        ),
                        true,
                    )
                })?;
            for path in [&self.jobs, &self.scripts, &self.claims] {
                fs::create_dir_all(path).await.map_err(|error| {
                    AppError::new(
                        "notification_unavailable",
                        format!("无法创建系统提醒目录 {}：{error}", path.display()),
                        true,
                    )
                })?;
                set_private_permissions(path, 0o700).await?;
            }
            Ok(())
        }
    }

    async fn set_private_permissions(path: &Path, mode: u32) -> Result<(), AppError> {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
            .await
            .map_err(|error| {
                AppError::new(
                    "notification_unavailable",
                    format!("无法设置提醒目录权限 {}：{error}", path.display()),
                    true,
                )
            })
    }

    async fn write_private_file(path: &Path, contents: &str, mode: u32) -> Result<(), AppError> {
        use std::os::unix::fs::PermissionsExt;

        fs::write(path, contents).await.map_err(|error| {
            AppError::new(
                "notification_unavailable",
                format!("无法写入系统提醒文件 {}：{error}", path.display()),
                true,
            )
        })?;
        fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
            .await
            .map_err(|error| {
                AppError::new(
                    "notification_unavailable",
                    format!("无法设置系统提醒文件权限 {}：{error}", path.display()),
                    true,
                )
            })
    }

    fn safe_component(value: &str) -> String {
        value
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                    character
                } else {
                    '_'
                }
            })
            .collect()
    }

    fn label_for(timer_id: &str) -> String {
        format!("{}{}", LABEL_PREFIX, safe_component(timer_id))
    }

    fn now_seconds() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or_default()
    }

    fn xml_escape(value: &str) -> String {
        value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;")
    }

    fn apple_script_escape(value: &str) -> String {
        value
            .chars()
            .map(|character| match character {
                '\\' => "\\\\".to_owned(),
                '"' => "\\\"".to_owned(),
                '\r' | '\n' | '\u{2028}' | '\u{2029}' => " ".to_owned(),
                other => other.to_string(),
            })
            .collect()
    }

    fn apple_script(title: &str) -> String {
        format!(
            "display notification \"{}\" with title \"爱丽丝的提醒\" sound name \"default\"\n",
            apple_script_escape(title)
        )
    }

    fn launch_agent_plist(spec: LaunchAgentSpec<'_>) -> String {
        let arguments = [
            "/bin/sh".to_owned(),
            spec.script_path.to_string_lossy().into_owned(),
            spec.claims_path.to_string_lossy().into_owned(),
            safe_component(spec.timer_id),
            spec.due_at_ms.to_string(),
            spec.label.to_owned(),
            spec.plist_path.to_string_lossy().into_owned(),
            spec.script_path.to_string_lossy().into_owned(),
            spec.apple_script_path.to_string_lossy().into_owned(),
            spec.uid.to_owned(),
        ];
        let argument_xml = arguments
            .iter()
            .map(|argument| format!("        <string>{}</string>\n", xml_escape(argument)))
            .collect::<String>();
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n    <key>Label</key>\n    <string>{label}</string>\n    <key>ProgramArguments</key>\n    <array>\n{argument_xml}    </array>\n    <key>RunAtLoad</key>\n    <true/>\n    <key>ProcessType</key>\n    <string>Background</string>\n    <key>LimitLoadToSessionType</key>\n    <string>Aqua</string>\n    <key>StandardOutPath</key>\n    <string>/dev/null</string>\n    <key>StandardErrorPath</key>\n    <string>/dev/null</string>\n</dict>\n</plist>\n",
            label = xml_escape(spec.label),
        )
    }

    fn shell_script() -> &'static str {
        r###"#!/bin/sh
set -eu

claims_dir="$1"
timer_id="$2"
due_at_ms="$3"
label="$4"
plist_path="$5"
script_path="$6"
apple_script_path="$7"
uid="$8"

due_seconds=$(( (due_at_ms + 999) / 1000 ))
now_seconds=$(/bin/date +%s)
remaining=$(( due_seconds - now_seconds ))
if [ "$remaining" -gt 0 ]; then
    /bin/sleep "$remaining"
fi

claim_path="$claims_dir/$timer_id"
if ! /bin/mkdir "$claim_path" 2>/dev/null; then
    exit 0
fi

/usr/bin/osascript "$apple_script_path" >/dev/null 2>&1 || true
/bin/launchctl bootout "gui/$uid/$label" >/dev/null 2>&1 || true
/bin/rm -f "$plist_path" "$script_path" "$apple_script_path"
"###
    }

    async fn current_uid() -> Result<String, AppError> {
        let output = Command::new("/usr/bin/id")
            .arg("-u")
            .stdin(Stdio::null())
            .output()
            .await
            .map_err(|error| {
                AppError::new(
                    "notification_unavailable",
                    format!("无法读取当前用户 ID：{error}"),
                    true,
                )
            })?;
        if !output.status.success() {
            return Err(AppError::new(
                "notification_unavailable",
                "读取当前用户 ID 失败。",
                true,
            ));
        }
        let uid = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if uid.is_empty() || !uid.chars().all(|character| character.is_ascii_digit()) {
            return Err(AppError::new(
                "notification_unavailable",
                "当前用户 ID 格式无效。",
                true,
            ));
        }
        Ok(uid)
    }

    async fn launchctl(args: &[String]) -> Result<std::process::Output, AppError> {
        Command::new("/bin/launchctl")
            .args(args)
            .stdin(Stdio::null())
            .output()
            .await
            .map_err(|error| {
                AppError::new(
                    "notification_unavailable",
                    format!("无法调用 macOS launchctl：{error}"),
                    true,
                )
            })
    }

    async fn remove_file_if_present(path: &Path) -> Result<(), AppError> {
        match fs::remove_file(path).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(AppError::new(
                "notification_unavailable",
                format!("无法清理系统提醒文件 {}：{error}", path.display()),
                true,
            )),
        }
    }

    async fn remove_dir_if_present(path: &Path) -> Result<(), AppError> {
        match fs::remove_dir(path).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) if error.kind() == ErrorKind::DirectoryNotEmpty => Ok(()),
            Err(error) => Err(AppError::new(
                "notification_unavailable",
                format!("无法清理系统提醒 claim {}：{error}", path.display()),
                true,
            )),
        }
    }

    fn job_paths(paths: &Paths, timer_id: &str) -> (String, PathBuf, PathBuf, PathBuf) {
        let label = label_for(timer_id);
        let plist_path = paths.launch_agents.join(format!("{label}.plist"));
        let script_path = paths.jobs.join(format!("{label}.sh"));
        let apple_script_path = paths.scripts.join(format!("{label}.applescript"));
        (label, plist_path, script_path, apple_script_path)
    }

    async fn bootout_and_remove_job(paths: &Paths, timer_id: &str) -> Result<(), AppError> {
        let uid = current_uid().await?;
        let (label, plist_path, script_path, apple_script_path) = job_paths(paths, timer_id);
        let target = format!("gui/{uid}/{label}");
        let _ = launchctl(&["bootout".into(), target]).await?;
        remove_file_if_present(&plist_path).await?;
        remove_file_if_present(&script_path).await?;
        remove_file_if_present(&apple_script_path).await
    }

    pub async fn schedule_timer(app: &AppHandle, timer: &TimerItem) -> Result<(), AppError> {
        let paths = Paths::for_app(app)?;
        paths.ensure().await?;
        let uid = current_uid().await?;
        let (label, plist_path, script_path, apple_script_path) = job_paths(&paths, &timer.id);
        let target = format!("gui/{uid}/{label}");
        let _ = launchctl(&["bootout".into(), target]).await?;
        write_private_file(&script_path, shell_script(), 0o700).await?;
        write_private_file(&apple_script_path, &apple_script(&timer.title), 0o600).await?;
        let plist = launch_agent_plist(LaunchAgentSpec {
            label: &label,
            script_path: &script_path,
            claims_path: &paths.claims,
            timer_id: &timer.id,
            due_at_ms: timer.due_at_ms,
            plist_path: &plist_path,
            apple_script_path: &apple_script_path,
            uid: &uid,
        });
        write_private_file(&plist_path, &plist, 0o600).await?;
        let output = launchctl(&[
            "bootstrap".into(),
            format!("gui/{uid}"),
            plist_path.display().to_string(),
        ])
        .await?;
        if !output.status.success() {
            let details = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            let _ = bootout_and_remove_job(&paths, &timer.id).await;
            return Err(AppError::new(
                "notification_unavailable",
                if details.is_empty() {
                    "macOS 没有接受这条持久提醒。".into()
                } else {
                    format!("macOS 没有接受这条持久提醒：{details}")
                },
                true,
            ));
        }
        Ok(())
    }

    pub async fn cancel_timer(app: &AppHandle, timer_id: &str) -> Result<(), AppError> {
        let paths = Paths::for_app(app)?;
        paths.ensure().await?;
        bootout_and_remove_job(&paths, timer_id).await?;
        let safe_id = safe_component(timer_id);
        remove_dir_if_present(&paths.claims.join(safe_id)).await
    }

    pub async fn cleanup_fired_job(app: &AppHandle, timer_id: &str) -> Result<(), AppError> {
        let paths = Paths::for_app(app)?;
        paths.ensure().await?;
        bootout_and_remove_job(&paths, timer_id).await
    }

    pub async fn claim_notification(app: &AppHandle, timer_id: &str) -> Result<bool, AppError> {
        let paths = Paths::for_app(app)?;
        paths.ensure().await?;
        let claim = paths.claims.join(safe_component(timer_id));
        match fs::create_dir(&claim).await {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => Ok(false),
            Err(error) => Err(AppError::new(
                "notification_unavailable",
                format!("无法建立提醒去重标记：{error}"),
                true,
            )),
        }
    }

    pub async fn reconcile(app: &AppHandle, timers: &[TimerItem]) -> Result<(), AppError> {
        let paths = Paths::for_app(app)?;
        paths.ensure().await?;
        let pending_ids = timers
            .iter()
            .map(|timer| safe_component(&timer.id))
            .collect::<HashSet<_>>();

        let mut entries = fs::read_dir(&paths.launch_agents).await.map_err(|error| {
            AppError::new(
                "notification_unavailable",
                format!("无法扫描 macOS 提醒任务：{error}"),
                true,
            )
        })?;
        while let Some(entry) = entries.next_entry().await.map_err(|error| {
            AppError::new(
                "notification_unavailable",
                format!("无法读取 macOS 提醒任务：{error}"),
                true,
            )
        })? {
            let file_name = entry.file_name().to_string_lossy().into_owned();
            if let Some(safe_id) = file_name
                .strip_prefix(LABEL_PREFIX)
                .and_then(|value| value.strip_suffix(".plist"))
            {
                if !pending_ids.contains(safe_id) {
                    bootout_and_remove_job(&paths, safe_id).await?;
                }
            }
        }

        let mut claims = fs::read_dir(&paths.claims).await.map_err(|error| {
            AppError::new(
                "notification_unavailable",
                format!("无法扫描提醒去重标记：{error}"),
                true,
            )
        })?;
        while let Some(entry) = claims.next_entry().await.map_err(|error| {
            AppError::new(
                "notification_unavailable",
                format!("无法读取提醒去重标记：{error}"),
                true,
            )
        })? {
            let safe_id = entry.file_name().to_string_lossy().into_owned();
            if !pending_ids.contains(&safe_id) {
                remove_dir_if_present(&entry.path()).await?;
            }
        }

        for timer in timers {
            let claim = paths.claims.join(safe_component(&timer.id));
            if fs::metadata(&claim).await.is_ok() {
                if timer.due_at_ms <= now_seconds() * 1_000 {
                    cleanup_fired_job(app, &timer.id).await?;
                    continue;
                }
                remove_dir_if_present(&claim).await?;
            }
            schedule_timer(app, timer).await?;
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn escapes_plist_and_applescript_content() {
            assert_eq!(xml_escape("a&<b>\"'"), "a&amp;&lt;b&gt;&quot;&apos;");
            assert_eq!(apple_script_escape("a\\\"\n"), "a\\\\\\\" ");
        }

        #[test]
        fn timer_job_script_has_absolute_due_and_atomic_claim() {
            let script = shell_script();
            assert!(script.contains("due_at_ms"));
            assert!(script.contains("/bin/mkdir \"$claim_path\""));
            assert!(script.contains("/usr/bin/osascript"));
        }
    }
}
