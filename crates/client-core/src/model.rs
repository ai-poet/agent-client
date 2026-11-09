//! Native-only provider availability for the daemon.

use std::path::Path;

pub use client_protocol::model::*;

pub fn provider_probe(provider: ProviderKind, _binary_override: Option<&str>) -> ProviderProbe {
    let installed = provider.is_builtin();
    ProviderProbe {
        provider,
        installed,
        path: None,
        models: if installed {
            crate::model_catalog::fallback_models(provider)
        } else {
            Vec::new()
        },
        agent_presets: Vec::new(),
    }
}

/// No CLI catalog cache is consulted: native models come from the selected
/// gateway origin and are synchronized by the desktop.
pub fn cached_provider_probe(
    provider: ProviderKind,
    binary_override: Option<&str>,
) -> ProviderProbe {
    provider_probe(provider, binary_override)
}

pub fn discover_provider_models(probe: ProviderProbe) -> ProviderProbe {
    probe
}

/// Read a tool's version for the legacy probe command. External provider probes
/// never reach this because they have no executable path.
pub fn probe_provider_version(binary: &Path) -> Option<String> {
    let mut command = crate::command_env::command(binary);
    let command = command.arg("--version").stdin(std::process::Stdio::null());
    let output = crate::command_env::output(command).ok()?;
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    parse_cli_version(&combined)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_builtin_provider_is_available() {
        assert!(provider_probe(ProviderKind::Native, None).installed);
        for provider in [
            ProviderKind::Amp,
            ProviderKind::Claude,
            ProviderKind::Codex,
            ProviderKind::Cursor,
            ProviderKind::DeepSeek,
            ProviderKind::Fx,
            ProviderKind::OpenCode,
            ProviderKind::Grok,
            ProviderKind::Kimi,
            ProviderKind::OhMyPi,
            ProviderKind::Pi,
        ] {
            let probe = provider_probe(provider, Some("/does/not/run"));
            assert!(!probe.installed);
            assert!(probe.path.is_none());
            assert!(probe.models.is_empty());
        }
    }
}
