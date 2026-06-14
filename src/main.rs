use anyhow::Result;
use clap::Parser;
use reshell::cli::{Cli, Commands};
use reshell::compact::view::{render_view, CompactView};
use reshell::memory::Store;
use reshell::sandbox::paths;

#[tokio::main]
async fn main() -> Result<()> {
    let args = Cli::parse();

    match args.command {
        Commands::Mcp { transport, port } => match transport.as_str() {
            "stdio" => {
                let server = reshell::mcp::McpServer::new()?;
                server.run().await?;
            }
            "sse" => {
                use std::net::SocketAddr;
                use std::sync::Arc;
                let addr: SocketAddr = ([127, 0, 0, 1], port).into();
                let store = reshell::memory::Store::new()?;
                let metrics = Arc::new(reshell::memory::metrics::Metrics::new());
                let router = Arc::new(reshell::mcp::Router::new(store, metrics));
                let sse_server = reshell::mcp::sse::SseServer::start(addr, router).await?;
                eprintln!(
                    "SSE MCP server listening on http://{}",
                    sse_server.local_addr()
                );
                std::future::pending::<()>().await;
            }
            _ => anyhow::bail!("Unknown transport: {}. Use 'stdio' or 'sse'.", transport),
        },
        Commands::Exec {
            command,
            cwd,
            timeout,
            retry,
            env,
            sandbox,
        } => {
            let runner = if sandbox {
                reshell::exec::runner::Runner::new_with_sandbox()?
            } else {
                reshell::exec::runner::Runner::new()?
            };
            let result = runner
                .run(&reshell::exec::ExecRequest {
                    command,
                    cwd,
                    timeout,
                    env: env.into_iter().collect(),
                    retry,
                })
                .await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Commands::Env => {
            let detector = reshell::env::Detector::cached().await;
            println!("{}", serde_json::to_string_pretty(&detector)?);
        }
        Commands::Completions { shell } => {
            reshell::completions::generate_completions(shell);
        }
        Commands::Compact {
            file,
            output_id,
            view,
            jq,
        } => {
            if let Some(path_expr) = jq.as_ref() {
                let content = if let Some(path) = file {
                    let (_validated, content) = paths::validate_and_read_file(&path)?;
                    content
                } else if let Some(output_id) = output_id {
                    let store = Store::new()?;
                    if let Some(output) = store.get_output(&output_id).await? {
                        output.stdout
                    } else {
                        anyhow::bail!("Unknown output_id: {}", output_id);
                    }
                } else {
                    anyhow::bail!("--jq requires --file or --output-id");
                };
                let extracted = reshell::compact::jq::extract_json_path(&content, path_expr)
                    .map_err(|e| anyhow::anyhow!("jq extraction failed: {}", e))?;
                println!("{}", extracted);
                return Ok(());
            }

            let store = Store::new()?;
            let view = CompactView::parse(&view);

            let result = if let Some(path) = file {
                let (_validated, content) = paths::validate_and_read_file(&path)?;
                render_view(&content, view, None, None)
            } else if let Some(output_id) = output_id {
                if let Some(output) = store.get_output(&output_id).await? {
                    let previous = if matches!(view, CompactView::Diff) {
                        store
                            .previous_output(&output.output_id)
                            .await?
                            .map(|previous| previous.stdout)
                    } else {
                        None
                    };
                    render_view(
                        &output.stdout,
                        view,
                        previous.as_deref(),
                        Some(output.output_id),
                    )
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
