use std::collections::HashSet;
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolCategory {
    ReadOnly,
    WriteFile,
    ShellCommand,
    ExternalNetwork,
    ProcessManagement,
}

impl ToolCategory {
    pub fn for_tool(name: &str) -> Self {
        match name {
            "read_file" | "list_directory" | "system_info" => ToolCategory::ReadOnly,
            "write_file" => ToolCategory::WriteFile,
            "shell" => ToolCategory::ShellCommand,
            "web_fetch" | "web_search" => ToolCategory::ExternalNetwork,
            _ => {
                if name.starts_with("mcp_") {
                    ToolCategory::ExternalNetwork
                } else {
                    ToolCategory::ShellCommand
                }
            }
        }
    }

    pub fn is_destructive(&self) -> bool {
        matches!(
            self,
            ToolCategory::WriteFile | ToolCategory::ShellCommand | ToolCategory::ProcessManagement
        )
    }

    pub fn description(&self) -> &str {
        match self {
            ToolCategory::ReadOnly => "read-only",
            ToolCategory::WriteFile => "file write",
            ToolCategory::ShellCommand => "shell command",
            ToolCategory::ExternalNetwork => "network request",
            ToolCategory::ProcessManagement => "process management",
        }
    }
}

#[derive(Debug, Clone)]
pub enum ApprovalDecision {
    Allow,
    Deny(String),
    AllowOnce,
    AllowAll(String),
}

pub type ApprovalFn = Box<dyn Fn(&str, &str, &str) -> ApprovalDecision + Send + Sync>;

pub struct ApprovalGate {
    approved_tools: Mutex<HashSet<String>>,
    policy: Mutex<ApprovalPolicy>,
    custom_handler: Mutex<Option<ApprovalFn>>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ApprovalPolicy {
    RequireAll,
    PromptDestructive,
    AllowAll,
}

impl Default for ApprovalPolicy {
    fn default() -> Self {
        ApprovalPolicy::PromptDestructive
    }
}

impl ApprovalGate {
    pub fn new() -> Self {
        Self {
            approved_tools: Mutex::new(HashSet::new()),
            policy: Mutex::new(ApprovalPolicy::PromptDestructive),
            custom_handler: Mutex::new(None),
        }
    }

    pub fn set_policy(&self, policy: ApprovalPolicy) {
        *self.policy.lock().unwrap() = policy;
    }

    pub fn set_handler<F: Fn(&str, &str, &str) -> ApprovalDecision + Send + Sync + 'static>(
        &self,
        handler: F,
    ) {
        *self.custom_handler.lock().unwrap() = Some(Box::new(handler));
    }

    pub fn needs_approval(&self, tool_name: &str, _args: &str) -> bool {
        let policy = *self.policy.lock().unwrap();
        if policy == ApprovalPolicy::AllowAll {
            return false;
        }
        if policy == ApprovalPolicy::PromptDestructive {
            let cat = ToolCategory::for_tool(tool_name);
            if !cat.is_destructive() {
                return false;
            }
        }
        let approved = self.approved_tools.lock().unwrap();
        !approved.contains(tool_name)
    }

    pub fn check(&self, tool_name: &str, args: &str, description: &str) -> Option<String> {
        let policy = *self.policy.lock().unwrap();
        if policy == ApprovalPolicy::AllowAll {
            return None;
        }

        let cat = ToolCategory::for_tool(tool_name);
        if policy == ApprovalPolicy::PromptDestructive && !cat.is_destructive() {
            return None;
        }

        let approved = self.approved_tools.lock().unwrap();
        if approved.contains(tool_name) {
            return None;
        }

        let handler = self.custom_handler.lock().unwrap();
        if let Some(ref h) = *handler {
            match h(tool_name, args, description) {
                ApprovalDecision::Allow | ApprovalDecision::AllowAll(_) => {
                    self.approved_tools.lock().unwrap().insert(tool_name.to_string());
                    None
                }
                ApprovalDecision::Deny(reason) => Some(reason),
                ApprovalDecision::AllowOnce => None,
            }
        } else {
            Some(format!(
                "Approval required: `{tool_name}` ({})\nArgs: {args}\nReply `approve {tool_name}` or `deny {tool_name}`.",
                cat.description(),
            ))
        }
    }

    pub fn approve_tool(&self, name: &str) {
        self.approved_tools.lock().unwrap().insert(name.to_string());
    }

    pub fn deny_tool(&self, name: &str) -> String {
        self.approved_tools.lock().unwrap().remove(name);
        format!("Denied: {name}. Reply `approve {name}` to retry.")
    }

    pub fn reset_approvals(&self) {
        self.approved_tools.lock().unwrap().clear();
    }
}
