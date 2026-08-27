//! Print the routing takeover state, CLI by CLI.
//!
//! ```bash
//! cargo run -p sub2api --example routing_doctor
//! ```
//!
//! Routing works by writing each CLI's own global configuration; this tool
//! shows what is currently desired (from the stored sign-in and custom
//! endpoints), what the takeover ledger says is managed, and whether each
//! live file actually carries our markers. Secrets are masked.

use sub2api::global_config::{self, PROVIDER_ID};

fn mask(secret: &str) -> String {
    let trimmed = secret.trim();
    if trimmed.len() <= 9 {
        return "***".to_owned();
    }
    format!("{}...{}", &trimmed[..6], &trimmed[trimmed.len() - 3..])
}

fn describe_target(target: Option<&global_config::RouteTarget>) -> String {
    match target {
        Some(target) => format!("{} key {}", target.base_url, mask(&target.api_key)),
        None => "unmanaged (user's own configuration)".to_owned(),
    }
}

fn file_marker(path: &std::path::Path, marker: &str) -> &'static str {
    match std::fs::read_to_string(path) {
        Ok(content) if content.contains(marker) => "MANAGED marker present",
        Ok(_) => "no marker",
        Err(_) => "file absent",
    }
}

fn main() {
    let Some(paths) = global_config::Paths::resolve() else {
        println!("!! could not resolve the home directory");
        return;
    };

    let credentials = sub2api::Credentials::load();
    let cloud = credentials.as_ref().map(|credentials| {
        sub2api::gateway_config_from(credentials, !credentials.routing_disabled)
    });
    match (&credentials, &cloud) {
        (Some(credentials), Some(config)) => println!(
            "cloud account: signed in to {} (routing {})",
            credentials.endpoint,
            if config.is_usable() { "on" } else { "off / no keys" }
        ),
        _ => println!("cloud account: signed out"),
    }
    let custom = sub2api::custom_api::load();
    println!(
        "custom endpoints file: {}",
        sub2api::custom_api::config_path()
            .map(|path| format!(
                "{} ({})",
                path.display(),
                if path.exists() { "present" } else { "absent" }
            ))
            .unwrap_or_else(|| "unresolvable".to_owned())
    );

    let desired = global_config::desired_routes(cloud.as_ref(), &custom);
    let state = global_config::load_state(&paths);

    println!("\n== desired vs ledger vs live ==");
    let rows: [(&str, Option<&global_config::RouteTarget>, bool, std::path::PathBuf, &str); 5] = [
        (
            "claude",
            desired.claude.as_ref(),
            state.claude.is_some(),
            paths.claude_dir.join("settings.json"),
            "ANTHROPIC_AUTH_TOKEN",
        ),
        (
            "codex",
            desired.codex.as_ref(),
            state.codex.is_some(),
            paths.codex_dir.join("config.toml"),
            PROVIDER_ID,
        ),
        (
            "grok",
            desired.grok.as_ref(),
            state.grok.is_some(),
            paths.grok_dir.join("config.toml"),
            "api_backend",
        ),
        (
            "opencode",
            desired.opencode.as_ref(),
            state.opencode,
            paths.opencode_dir.join("opencode.json"),
            PROVIDER_ID,
        ),
        (
            "pi",
            desired.pi.as_ref(),
            state.pi,
            paths.pi_agent_dir.join("models.json"),
            PROVIDER_ID,
        ),
    ];
    for (cli, target, managed, path, marker) in rows {
        println!("  {cli}:");
        println!("    desired: {}", describe_target(target));
        println!("    ledger:  {}", if managed { "managed" } else { "not managed" });
        println!("    live:    {} — {}", path.display(), file_marker(&path, marker));
    }

    println!("\nA mismatch between desired and live means reconcile has not run since");
    println!("the state changed — sign in/out, switch a group, or save an endpoint in");
    println!("the app to trigger it.");
}
