use crate::{
    runtime::types::{RuntimeToolCall, RuntimeToolResult},
    tools_runtime::{ToolDefinition, ToolExecutor},
};
use async_trait::async_trait;
use std::collections::BTreeMap;

pub struct CompositeToolExecutor {
    executors: Vec<Box<dyn ToolExecutor>>,
}

impl CompositeToolExecutor {
    pub fn new(executors: Vec<Box<dyn ToolExecutor>>) -> Self {
        Self { executors }
    }
}

#[async_trait]
impl ToolExecutor for CompositeToolExecutor {
    async fn definitions(&self) -> Vec<ToolDefinition> {
        let mut definitions = BTreeMap::new();
        for executor in &self.executors {
            for definition in executor.definitions().await {
                definitions.insert(definition.name.clone(), definition);
            }
        }
        definitions.into_values().collect()
    }

    async fn execute(&self, call: &RuntimeToolCall) -> RuntimeToolResult {
        for executor in &self.executors {
            if executor
                .definitions()
                .await
                .iter()
                .any(|definition| definition.name == call.name)
            {
                return executor.execute(call).await;
            }
        }

        RuntimeToolResult {
            tool_call_id: call.id.clone(),
            name: call.name.clone(),
            content: format!("Tool not found: {}", call.name),
            is_error: true,
        }
    }
}
