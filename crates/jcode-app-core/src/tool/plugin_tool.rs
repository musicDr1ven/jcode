//! A tool offered by a plugin, exposed to the model through the ordinary
//! dynamic `Tool` registry (design D5).
//!
//! The model-facing name is `<plugin>__<tool>`; calls are forwarded to the
//! owning plugin with the plugin's own tool name.

use serde_json::Value;

use crate::plugins::PluginToolSpec;
use crate::tool::{Tool, ToolContext, ToolOutput};

pub(crate) struct PluginTool {
    spec: PluginToolSpec,
}

impl PluginTool {
    pub(crate) fn new(spec: PluginToolSpec) -> Self {
        Self { spec }
    }
}

#[async_trait::async_trait]
impl Tool for PluginTool {
    fn name(&self) -> &str {
        &self.spec.flattened_name
    }

    fn description(&self) -> &str {
        &self.spec.description
    }

    fn parameters_schema(&self) -> Value {
        self.spec.input_schema.clone()
    }

    async fn execute(
        &self,
        input: Value,
        ctx: ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let cwd = ctx
            .working_dir
            .as_ref()
            .map(|dir| dir.display().to_string());
        let output = crate::plugins::call_plugin_tool(
            &ctx.session_id,
            cwd.as_deref(),
            &self.spec.flattened_name,
            input,
        )
        .await?;
        Ok(ToolOutput::new(output).with_title(format!("plugin: {}", self.spec.plugin)))
    }
}
