# Command Central

A self-hosted AI agent terminal and Discord bot for coding, system administration, and automation. Powered by any OpenAI-compatible LLM with function-calling support.

## Features

- **AI Agent** — Autonomous coding assistant with shell access, file I/O, code search, web browsing, and MCP (Model Context Protocol) server integration
- **Discord Bot** — Control your server from Discord with the same AI agent, plus direct commands for monitoring, scheduling, and task execution
- **Interactive TUI** — Terminal UI built with ratatui + crossterm with non-blocking agent calls
- **REPL** — Command-line REPL for quick shell commands and AI queries
- **Task Queue** — Background task execution with SQLite persistence (shell commands, SIF scans, NMAP, email, opencode integration)
- **Scheduled Loops** — Recurring shell commands on configurable intervals
- **System Monitoring** — Process detection, service health checks, system resource reporting
- **Agent Detection** — Scans host for installed coding agents and dev tools
- **MCP Support** — Connect to any MCP-compatible tool server (filesystem, GitHub, databases, cloud, browser automation, and 20+ more)

## Quick Start

### Prerequisites

- Rust toolchain (install via [rustup](https://rustup.rs))
- An OpenAI-compatible API key (OpenAI, Anthropic via proxy, local LLM, etc.)

### Install

```bash
git clone https://github.com/your-org/command_central.git
cd command_central
cargo build --release
```

### Configure

Run the setup wizard:

```bash
./target/release/command_central setup
```

Or create `config.toml` manually:

```toml
[llm]
provider = "openai"
api_key = "sk-..."
model = "gpt-4"
base_url = "https://api.openai.com/v1"

[discord]
token = "your-bot-token"
# channel_id = "123456789"  # optional: restrict to one channel

[paths]
atomic_repo = "$HOME/Atomic"
opencode_bin = "$HOME/.opencode/bin/opencode"

[mcp]
servers = []
```

### Run

```bash
# Discord bot (default mode)
command_central discord

# Direct shell command
command_central shell "git status"

# Ask the AI agent
command_central ask "What's using the most disk space?"

# Interactive TUI
command_central tui

# REPL mode
command_central repl

# View config
command_central config show
```

## Discord Commands

| Command | Description |
|---------|-------------|
| `run <cmd>` | Execute a shell command |
| `agents` | List detected coding agents & tools |
| `services` | List running systemd services |
| `monitor p <name>` | Find processes matching name |
| `monitor s <name>` | Check systemd service status |
| `loop <n> = <c> every <s>` | Create scheduled command |
| `workon <task>` | Run opencode on Atomic repo |
| `history [n]` | Show task history |
| `cancel <id>` | Cancel a running task |
| `cancel` | Cancel current agent operation |
| `approve <tool>` | Approve pending tool execution |
| `deny <tool>` | Deny pending tool execution |
| `delete <id>` | Delete a task |
| `retry <id>` | Retry a failed task |
| `reset` | Reset conversation with agent |
| `config show/set/mcp` | View or edit configuration |

Anything else is routed to the AI agent with full tool access.

## AI Agent Tools

| Tool | Description |
|------|-------------|
| `shell` | Execute any shell command |
| `read_file` | Read file contents |
| `write_file` | Write/overwrite files |
| `grep_search` | Regex search in files |
| `system_info` | Get system resource info |
| `list_directory` | List directory contents |
| `web_fetch` | Fetch and parse web pages |
| `web_search` | Search via DuckDuckGo |

Plus any MCP server tools you configure.

## MCP Servers

Configure MCP servers in `config.toml`:

```toml
[mcp]
servers = [
  { name = "filesystem", command = "npx", args = ["-y", "@modelcontextprotocol/server-filesystem", "/home/user"] },
  { name = "github", command = "npx", args = ["-y", "@modelcontextprotocol/server-github"] },
]
```

The example config includes commented-out templates for 20+ MCP servers.

## Safety

- **Approval Gate** — Destructive tool calls (shell commands, file writes) require confirmation by default. Configure with `approve <tool>` / `deny <tool>`.
- **Cancellation** — Ctrl+C in TUI, `cancel` in Discord, `stop` in Discord all interrupt in-flight agent operations.
- **System Prompt** — The agent is instructed to warn before destructive operations and never expose secrets.

## Architecture

```
src/
  main.rs          Entry point, CLI parsing, subcommand dispatch
  agent/           LLM client, tool definitions, approval gate, chat sessions
  core/            Config, database, executor, MCP client, monitor, scheduler
  cli/             TUI (ratatui), REPL
  discord/         Discord bot (serenity)
  utils/           Shell command execution
```

## Development

```bash
cargo test                    # Run tests
cargo build                   # Dev build
cargo build --release         # Release build (LTO + strip)
cargo clippy -- -D warnings   # Lint
cargo fmt -- --check          # Format check
```
