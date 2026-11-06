//! Drives a real turn against claude-agent-acp through DesktopSession.
//!
//! Usage:
//!   cargo run --example claude_chat -- "你的问题"
//!
//! Requires the `claude-agent-acp` adapter on PATH and valid Claude
//! credentials (ANTHROPIC_API_KEY, or an existing Claude Code login the
//! adapter can reuse).
use longbridge_ai_acp::acp::schema::v1::{ContentBlock, SessionUpdate};
use longbridge_ai_acp::{
    AllowlistedTools, DesktopSession, DesktopSessionEvent, ExternalAgent, ExternalAgentKind,
};
use std::io::Write;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let question = std::env::args().nth(1).unwrap_or_else(|| {
        // "使用 algo-terminal skills 统计本月 bailian 渠道各模型的使用费用".to_string()
        "列出当前文件夹 /Users/yangyang/lab/rust/longbridge-ai-acp 中共有多少子文件夹".to_string()
        // "查看当前 git 最近一次提交记录".to_string()
        // "查看目录 /Users/yangyang/changqiao/bi-nodesrv-search 中最近一次提交记录".to_string()
        // "使用 navlane mcp 查看 bi-nodesrv-search 最近的日志？（使用中文回答这个问题）".to_string()
        // "使用 navlane mcp 都支持哪些集群？（使用中文回答）".to_string()
    });

    // Launch claude-agent-acp as an external ACP subprocess.
    let config = ExternalAgentKind::Claude.config();
    let mut session = DesktopSession::connect(
        ExternalAgent::new(config),
        // std::env::current_dir()?,
        std::path::PathBuf::from("/tmp"),
        Arc::new(AllowlistedTools),
    )
    .await?;

    eprintln!("connected to: {:?}", session.handshake().implementation);
    // eprintln!("question: {question}\n--- answer ---");

    session.prompt(question).await?;

    while let Some(event) = session.next_event().await {
        match event {
            DesktopSessionEvent::Update(update) => match *update {
                SessionUpdate::AgentMessageChunk(chunk) => {
                    if let ContentBlock::Text(text) = chunk.content {
                        print!("{}", text.text);
                        std::io::stdout().flush().ok();
                    }
                }
                SessionUpdate::AgentThoughtChunk(chunk) => {
                    eprintln!("\n[Thoughts: ] ");
                    if let ContentBlock::Text(text) = chunk.content {
                        eprint!("{}", text.text);
                        std::io::stderr().flush().ok();
                    }
                }
                SessionUpdate::AvailableCommandsUpdate(_) => eprintln!("AvailableCommandsUpdate"),
                SessionUpdate::UsageUpdate(_) => eprintln!("UsageUpdate"),
                other => eprintln!("\n[update] {other:?}"),
            },
            DesktopSessionEvent::TurnFinished(reason) => {
                eprintln!("\n--- turn finished: {reason:?} ---");
                break;
            }
            DesktopSessionEvent::Failed(error) => {
                eprintln!("\nfailed: {error}");
                break;
            }
            _ => {}
        }
    }

    session.shutdown().await;
    Ok(())
}
