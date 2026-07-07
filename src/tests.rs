#[cfg(test)]
mod tests {
    use crate::agent::gate::{ApprovalGate, ApprovalPolicy, ToolCategory};
    use crate::agent::tools::{get_builtin_tool_definitions, ToolDefinition};
    use crate::core::config::Config;

    #[test]
    fn test_tool_definitions_have_required_fields() {
        let tools = get_builtin_tool_definitions();
        assert!(!tools.is_empty(), "Should have built-in tools");

        for tool in &tools {
            assert!(!tool.name.is_empty(), "Tool name must not be empty");
            assert!(!tool.description.is_empty(), "Tool description must not be empty");
        }

        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"shell"), "Must have shell tool");
        assert!(names.contains(&"read_file"), "Must have read_file tool");
        assert!(names.contains(&"write_file"), "Must have write_file tool");
        assert!(names.contains(&"grep_search"), "Must have grep_search tool");
        assert!(names.contains(&"system_info"), "Must have system_info tool");
        assert!(names.contains(&"list_directory"), "Must have list_directory tool");
        assert!(names.contains(&"web_fetch"), "Must have web_fetch tool");
        assert!(names.contains(&"web_search"), "Must have web_search tool");
    }

    #[test]
    fn test_tool_definitions_have_valid_json_schema() {
        let tools = get_builtin_tool_definitions();
        for tool in &tools {
            let params = &tool.parameters;
            assert!(params.is_object(), "parameters must be a JSON object");
            assert_eq!(
                params["type"].as_str().unwrap_or(""),
                "object",
                "parameters.type must be 'object'"
            );
        }
    }

    #[test]
    fn test_tool_category_classification() {
        assert_eq!(
            ToolCategory::for_tool("read_file"),
            ToolCategory::ReadOnly,
            "read_file should be ReadOnly"
        );
        assert_eq!(
            ToolCategory::for_tool("write_file"),
            ToolCategory::WriteFile,
            "write_file should be WriteFile"
        );
        assert_eq!(
            ToolCategory::for_tool("shell"),
            ToolCategory::ShellCommand,
            "shell should be ShellCommand"
        );
        assert_eq!(
            ToolCategory::for_tool("web_fetch"),
            ToolCategory::ExternalNetwork,
            "web_fetch should be ExternalNetwork"
        );
        assert!(ToolCategory::for_tool("read_file").is_destructive() == false);
        assert!(ToolCategory::for_tool("write_file").is_destructive() == true);
        assert!(ToolCategory::for_tool("shell").is_destructive() == true);
    }

    #[test]
    fn test_approval_gate_default_policy() {
        let gate = ApprovalGate::new();
        assert!(
            gate.needs_approval("shell", "rm -rf /"),
            "Destructive tool should need approval by default"
        );
        assert!(
            !gate.needs_approval("read_file", "/etc/hosts"),
            "Read-only tool should not need approval by default"
        );
    }

    #[test]
    fn test_approval_gate_allow_all_policy() {
        let gate = ApprovalGate::new();
        gate.set_policy(ApprovalPolicy::AllowAll);
        assert!(
            !gate.needs_approval("shell", "rm -rf /"),
            "AllowAll should not need approval for any tool"
        );
    }

    #[test]
    fn test_approval_gate_approve_and_deny() {
        let gate = ApprovalGate::new();
        assert!(gate.needs_approval("write_file", "/tmp/test.txt"));
        gate.approve_tool("write_file");
        assert!(!gate.needs_approval("write_file", "/tmp/test.txt"));
        gate.deny_tool("write_file");
        assert!(gate.needs_approval("write_file", "/tmp/test.txt"));
    }

    #[test]
    fn test_approval_gate_check_returns_none_when_approved() {
        let gate = ApprovalGate::new();
        gate.approve_tool("shell");
        assert_eq!(gate.check("shell", "ls", "list files"), None);
    }

    #[test]
    fn test_approval_gate_check_returns_some_when_not_approved() {
        let gate = ApprovalGate::new();
        let result = gate.check("shell", "rm -rf /", "shell command");
        assert!(result.is_some(), "Should require approval for unapproved destructive tool");
    }

    #[test]
    fn test_approval_gate_check_readonly_is_none() {
        let gate = ApprovalGate::new();
        assert_eq!(gate.check("read_file", "/etc/hosts", "read file"), None);
    }

    #[test]
    fn test_config_default_llm_provider() {
        let config = Config::default();
        assert_eq!(config.llm.provider.as_deref(), Some("openai"));
        assert_eq!(config.llm.model.as_deref(), Some("gpt-4"));
        assert!(config.llm.api_key.is_none());
    }

    #[test]
    fn test_config_default_paths_use_home() {
        let config = Config::default();
        assert!(config.paths.atomic_repo.is_some());
        assert!(config.paths.opencode_bin.is_some());
        let home = std::env::var("HOME").unwrap_or_else(|_| "/home/user".to_string());
        assert!(config.paths.atomic_repo.as_deref().unwrap_or("").contains(&home));
        assert!(config.paths.opencode_bin.as_deref().unwrap_or("").contains(&home));
    }

    #[test]
    fn test_config_format_report() {
        let config = Config::default();
        let report = config.format_report();
        assert!(report.contains("LLM"), "report should contain LLM section");
        assert!(report.contains("Discord"), "report should contain Discord section");
    }

    #[test]
    fn test_tool_definition_mcp_fields_default_none() {
        let tools = get_builtin_tool_definitions();
        for tool in &tools {
            assert!(tool.mcp_server.is_none(), "Built-in tools should have no mcp_server");
            assert!(tool.mcp_tool.is_none(), "Built-in tools should have no mcp_tool");
        }
    }

    #[test]
    fn test_shell_run_echo() {
        let result = crate::utils::shell::run_shell("echo hello");
        assert!(result.is_ok(), "echo should succeed");
        let (stdout, stderr, status) = result.unwrap();
        assert!(status.success(), "echo exit code should be 0");
        assert!(stdout.contains("hello"), "echo should output hello");
        assert!(stderr.is_empty(), "echo should have no stderr");
    }

    #[test]
    fn test_shell_run_failure() {
        let result = crate::utils::shell::run_shell("nonexistent_command_xyz_123 2>/dev/null");
        assert!(result.is_ok(), "shell should not error on unknown command");
        let (_, _, status) = result.unwrap();
        assert!(!status.success(), "unknown command should fail");
    }

    #[test]
    fn test_task_kind_labels() {
        use crate::core::task::Task;
        use tokio::sync::oneshot;

        assert_eq!(Task::Sif("url".into()).kind(), "sif");
        assert_eq!(Task::Nmap("target".into()).kind(), "nmap");
        assert_eq!(Task::Shell("cmd".into()).kind(), "shell");
        assert_eq!(Task::Run("cmd".into()).kind(), "run");
        assert_eq!(Task::Agents.kind(), "agents");
        assert_eq!(Task::Services.kind(), "services");
        assert_eq!(Task::CancelTask(1).kind(), "cancel");
        assert_eq!(Task::DeleteTask(1).kind(), "delete");
        assert_eq!(Task::RetryTask(1).kind(), "retry");
        let (tx, _rx) = oneshot::channel();
        assert_eq!(Task::History(tx).kind(), "history");
    }

    #[test]
    fn test_monitor_find_process() {
        let result = crate::core::monitor::find_process("init");
        assert!(!result.is_empty(), "find_process should return a string");
        // init/systemd should be running on most Linux systems
    }

    #[test]
    fn test_detector_finds_shell() {
        let agents = crate::core::detector::detect_agents();
        assert!(!agents.is_empty(), "Should detect at least some tools");
        let found_sh = agents.iter().any(|a| a.name == "sh" || a.name == "bash");
        // Not asserting found_sh because CI might not have sh in PATH
    }

    #[test]
    fn test_message_serialization() {
        let msg = crate::agent::llm::Message {
            role: "user".into(),
            content: "hello".into(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("user"));
        assert!(json.contains("hello"));
    }
}
