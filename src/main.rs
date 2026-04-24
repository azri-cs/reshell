use reshell::cli::{Cli, Commands};
use clap::Parser;
use anyhow::Result;
use reshell::compact::view::{CompactView, render_view};
use reshell::memory::Store;

#[tokio::main]
async fn main() -> Result<()> {
    let args = Cli::parse();

    match args.command {
        Commands::Mcp => {
            let server = reshell::mcp::McpServer::new();
            server.run().await?;
        }
        Commands::Exec { command, cwd, timeout, retry, env } => {
            let runner = reshell::exec::runner::Runner::new()?;
            let result = runner.run(&reshell::exec::ExecRequest {
                command,
                cwd,
                timeout,
                env: env.into_iter().collect(),
                retry,
            }).await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Commands::Env => {
            let detector = reshell::env::Detector::cached().await;
            println!("{}", serde_json::to_string_pretty(&detector)?);
        }
        Commands::Compact { file, output_id, view } => {
            let store = Store::new()?;
            let view = CompactView::parse(&view);

            let result = if let Some(path) = file {
                let content = tokio::fs::read_to_string(&path).await?;
                render_view(&content, view, None, None)
            } else if let Some(output_id) = output_id {
                if let Some(output) = store.get_output(&output_id)? {
                    let previous = if matches!(view, CompactView::Diff) {
                        store.previous_output(&output.output_id)?
                            .map(|previous| previous.stdout)
                    } else {
                        None
                    };
                    render_view(&output.stdout, view, previous.as_deref(), Some(output.output_id))
                } else {
                    anyhow::bail!("Unknown output_id: {}", output_id);
                }
            } else {
                anyhow::bail!("Error: --file or --output-id required for compact");
            };

            println!("{}", serde_json::to_string_pretty(&result)?);
        }
    }

    Ok(())
}
