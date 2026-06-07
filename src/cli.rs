use clap::{Parser, Subcommand};
#[derive(Parser, Debug)]
#[command(name = "rsh")]
#[command(about = "Resilient Shell Execution Middleware for AI Agents")]
#[command(version = env!("CARGO_PKG_VERSION"))]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Run as an MCP server
    Mcp {
        /// Transport mode: stdio (default) or sse
        #[arg(long, default_value = "stdio")]
        transport: String,

        /// Port for SSE transport (default: 3000)
        #[arg(long, default_value_t = 3000)]
        port: u16,
    },
    /// Execute a command directly (CLI mode)
    Exec {
        #[arg(short = 'c', long)]
        command: String,
        #[arg(short = 'C', long)]
        cwd: Option<String>,
        #[arg(short = 't', long, default_value_t = 120)]
        timeout: u64,
        #[arg(long, default_value_t = true)]
        retry: bool,
        #[arg(short = 'E', long, value_parser = parse_key_val)]
        env: Vec<(String, String)>,
        #[arg(long, default_value_t = false)]
        sandbox: bool,
    },
    /// Detect and describe the current shell environment
    Env,
    /// Generate shell completion scripts
    Completions {
        /// Shell: bash, zsh, fish, elvish, or powershell
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
    /// Compact a file or previous output
    Compact {
        #[arg(short, long)]
        file: Option<String>,
        #[arg(short, long)]
        output_id: Option<String>,
        #[arg(short, long, default_value = "skeleton")]
        view: String,
        #[arg(long)]
        jq: Option<String>,
    },
}

fn parse_key_val(s: &str) -> Result<(String, String), String> {
    let pos = s
        .find('=')
        .ok_or_else(|| format!("invalid KEY=value: no `=` found in `{}`", s))?;
    Ok((s[..pos].to_string(), s[pos + 1..].to_string()))
}
