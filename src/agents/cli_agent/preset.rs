pub(super) struct CliPreset {
    pub(super) command: &'static str,
    pub(super) args: &'static [&'static str],
    pub(super) prompt_stdin: bool,
    pub(super) append_prompt: bool,
}

impl CliPreset {
    pub(super) fn for_agent(name: &str) -> Self {
        match name {
            "claudecode" | "claude-code" | "claude" => Self {
                command: "claude",
                args: &["-p", "{prompt}"],
                prompt_stdin: false,
                append_prompt: false,
            },
            "cursor" | "cursor-agent" => Self {
                command: "agent",
                args: &["--print", "--output-format", "stream-json"],
                prompt_stdin: true,
                append_prompt: false,
            },
            "gemini" | "gemini-cli" => Self {
                command: "gemini",
                args: &["-p", "--output-format", "stream-json"],
                prompt_stdin: true,
                append_prompt: false,
            },
            "qoder" | "qoder-cli" => Self {
                command: "qodercli",
                args: &["-p", "{prompt}", "-f", "stream-json"],
                prompt_stdin: false,
                append_prompt: false,
            },
            "opencode" => Self {
                command: "opencode",
                args: &["run", "--format", "json"],
                prompt_stdin: true,
                append_prompt: false,
            },
            "iflow" | "iflow-cli" => Self {
                command: "iflow",
                args: &["-i"],
                prompt_stdin: true,
                append_prompt: false,
            },
            "kimi" | "kimi-cli" => Self {
                command: "kimi",
                args: &["--print", "--output-format", "stream-json"],
                prompt_stdin: true,
                append_prompt: false,
            },
            "pi" => Self {
                command: "pi",
                args: &["{prompt}"],
                prompt_stdin: false,
                append_prompt: false,
            },
            "devin" => Self {
                command: "devin",
                args: &["acp"],
                prompt_stdin: true,
                append_prompt: false,
            },
            _ => Self {
                command: "agent",
                args: &[],
                prompt_stdin: true,
                append_prompt: false,
            },
        }
    }

    pub(super) fn args(&self) -> Vec<String> {
        self.args.iter().map(|arg| arg.to_string()).collect()
    }
}
