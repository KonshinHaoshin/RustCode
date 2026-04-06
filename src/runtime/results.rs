use crate::runtime::types::{RuntimeMessage, RuntimeToolCall};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeUsage {
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    pub total_tokens: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingApproval {
    pub tool_call: RuntimeToolCall,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnStatus {
    Completed,
    AwaitingApproval,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalAction {
    AllowOnce(PendingApproval),
    DenyOnce(PendingApproval),
    AlwaysAllow(PendingApproval),
    AlwaysDeny(PendingApproval),
}

#[derive(Debug, Clone)]
pub struct QueryTurnResult {
    pub history: Vec<RuntimeMessage>,
    pub assistant_message: Option<RuntimeMessage>,
    pub usage: Option<RuntimeUsage>,
    pub model: String,
    pub finish_reason: Option<String>,
    pub tool_call_count: usize,
    pub status: TurnStatus,
    pub pending_approval: Option<PendingApproval>,
}

impl QueryTurnResult {
    pub fn assistant_text(&self) -> Option<&str> {
        self.assistant_message
            .as_ref()
            .map(|message| message.content.as_str())
    }
}
