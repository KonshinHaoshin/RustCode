use crate::session::SessionPlan;

pub fn render_session_plan(plan: &SessionPlan) -> String {
    plan.trim().to_string()
}

pub fn plan_help_text() -> String {
    "/plan enables plan mode. Once enabled, normal prompts are handled by the built-in Plan Agent.\n/plan while already in plan mode shows the current session plan.\n/plan open is reserved for opening the current plan in an external editor.".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_raw_plan_text() {
        let plan = "# Current Plan\n\n- inspect runtime\n- update slash command".to_string();
        assert_eq!(render_session_plan(&plan), plan);
    }
}
