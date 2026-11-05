use super::*;

impl Waku {
    pub(super) fn export_daemon_diagnostics(&self, cx: &mut Context<Self>) {
        let client = self.daemon.client();
        let metadata = waku_client::DaemonDiagnosticsMetadata {
            exported_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            app_version: env!("CARGO_PKG_VERSION").into(),
            app_pid: std::process::id(),
            daemon_version: client.daemon_version().to_owned(),
            daemon_pid: self.daemon.daemon_pid(),
            connection_id: client.connection_id().to_owned(),
            remote_daemon: self.daemon.is_remote(),
        };
        let directory = dirs::document_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(std::env::temp_dir);
        let suggested_name = format!(
            "waku-daemon-diagnostics-{}.zip",
            chrono::Utc::now().format("%Y%m%d-%H%M%S")
        );
        let receiver = cx.prompt_for_new_path(&directory, Some(&suggested_name));
        cx.spawn(async move |this, cx| {
            let path = match receiver.await {
                Ok(Ok(Some(path))) => path,
                Ok(Ok(None)) => return,
                Ok(Err(error)) => {
                    let _ = this.update(cx, |this, cx| {
                        this.show_toast(tr!("daemon.diagnostics.export_failed", error = error));
                        cx.notify();
                    });
                    return;
                }
                Err(error) => {
                    let _ = this.update(cx, |this, cx| {
                        this.show_toast(tr!("daemon.diagnostics.export_failed", error = error));
                        cx.notify();
                    });
                    return;
                }
            };
            let result = cx
                .background_executor()
                .spawn(async move {
                    waku_client::validate_destination(&path)?;
                    waku_client::export_daemon_diagnostics(&path, &metadata)?;
                    Ok::<_, anyhow::Error>(path)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(path) => this.show_success_toast(tr!(
                        "daemon.diagnostics.exported",
                        path = path.display().to_string()
                    )),
                    Err(error) => this.show_toast(tr!(
                        "daemon.diagnostics.export_failed",
                        error = format!("{error:#}")
                    )),
                }
                cx.notify();
            });
        })
        .detach();
    }
}
