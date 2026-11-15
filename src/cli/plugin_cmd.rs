//! `jcode plugin list` / `jcode plugin test`: inspect configured plugins
//! without starting a session or calling a model.

use anyhow::Result;
use serde_json::json;

use crate::plugins::PluginProbeReport;

/// List every configured plugin and whether it is currently usable. Usability
/// is established by actually starting the plugin and completing the handshake,
/// so a broken entry is reported as broken rather than merely "configured".
pub(crate) async fn run_list(json_output: bool) -> Result<()> {
    let plugins = crate::config::config().plugins.clone();
    if plugins.is_empty() {
        if json_output {
            println!("{}", serde_json::to_string_pretty(&json!({ "plugins": [] }))?);
        } else {
            println!(
                "No plugins configured. Add a [[plugins]] table to config.toml; see docs/PLUGINS.md."
            );
        }
        return Ok(());
    }

    let mut reports = Vec::with_capacity(plugins.len());
    for plugin in &plugins {
        reports.push(crate::plugins::probe_plugin(plugin).await);
    }

    if json_output {
        let values: Vec<_> = reports.iter().map(report_json).collect();
        println!("{}", serde_json::to_string_pretty(&values)?);
    } else {
        for report in &reports {
            println!("{}", human_line(report));
        }
    }
    Ok(())
}

/// Test one configured plugin: start it, complete the handshake, report its
/// version and capabilities, and stop it.
pub(crate) async fn run_test(name: &str, json_output: bool) -> Result<()> {
    let plugin = crate::config::config()
        .plugins
        .iter()
        .find(|plugin| plugin.name == name)
        .cloned();
    let Some(plugin) = plugin else {
        anyhow::bail!(
            "No plugin named '{name}'. Run `jcode plugin list` to see the configured plugins."
        );
    };

    let report = crate::plugins::probe_plugin(&plugin).await;
    if json_output {
        println!("{}", serde_json::to_string_pretty(&report_json(&report))?);
    } else {
        println!("{}", human_line(&report));
        if report.usable {
            println!(
                "  protocol: v{}",
                report
                    .protocol
                    .map(|version| version.to_string())
                    .unwrap_or_else(|| "?".to_string())
            );
            println!(
                "  version:  {}",
                report.version.as_deref().unwrap_or("unknown")
            );
            println!(
                "  declared: {}",
                capability_list(&report.declared)
            );
            println!(
                "  granted:  {}",
                capability_list(&report.granted)
            );
        }
    }

    if !report.usable {
        anyhow::bail!(
            "plugin '{name}' failed its test: {}",
            report.error.unwrap_or_else(|| "unknown failure".to_string())
        );
    }
    Ok(())
}

fn human_line(report: &PluginProbeReport) -> String {
    if report.usable {
        format!(
            "✓ {:<20} {} v{} (protocol v{}), granted: {}",
            report.name,
            report.reported_name.as_deref().unwrap_or(&report.name),
            report.version.as_deref().unwrap_or("unknown"),
            report
                .protocol
                .map(|version| version.to_string())
                .unwrap_or_else(|| "?".to_string()),
            capability_list(&report.granted),
        )
    } else {
        format!(
            "✗ {:<20} unusable: {}",
            report.name,
            report.error.as_deref().unwrap_or("unknown failure")
        )
    }
}

fn capability_list(capabilities: &[crate::plugins::PluginCapability]) -> String {
    if capabilities.is_empty() {
        return "(none)".to_string();
    }
    capabilities
        .iter()
        .map(|capability| capability.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn report_json(report: &PluginProbeReport) -> serde_json::Value {
    json!({
        "name": report.name,
        "usable": report.usable,
        "reported_name": report.reported_name,
        "version": report.version,
        "protocol": report.protocol,
        "declared": report.declared.iter().map(|c| c.as_str()).collect::<Vec<_>>(),
        "granted": report.granted.iter().map(|c| c.as_str()).collect::<Vec<_>>(),
        "error": report.error,
    })
}
