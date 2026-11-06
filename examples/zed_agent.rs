//! A minimal Longbridge ACP *agent* that Zed (an ACP client) can connect to.
//!
//! This is the opposite direction of `claude_chat`: here the program is the
//! **Agent end** (`serve_stdio`), and Zed is the client. Replace the body of
//! `MyBackend::prompt` with real Longbridge AI calls to ship a real agent.
//!
//! Build it, then point Zed's `agent_servers` at the compiled binary:
//!
//!   cargo build --release --example zed_agent
//!   # binary at: target/release/examples/zed_agent
//!
//! settings.json (~/.config/zed/settings.json):
//!   {
//!     "agent_servers": {
//!       "Longbridge AI": {
//!         "command": "/ABS/PATH/target/release/examples/zed_agent",
//!         "args": [],
//!         "env": {}
//!       }
//!     }
//!   }
//!
//! IMPORTANT: stdout is the ACP protocol channel. Never `println!` here — all
//! diagnostics must go to stderr (`eprintln!` / tracing to stderr).
use async_trait::async_trait;
use futures::stream::{self, BoxStream};
use longbridge_ai_acp::{serve_stdio, AgentBackend, AgentEvent, BackendError, Prompt};
use std::path::Path;

/// Replace this with your real Longbridge API client / auth / session state.
struct MyBackend;

#[async_trait]
impl AgentBackend for MyBackend {
    type Session = ();

    async fn prompt(
        &self,
        _session: Self::Session,
        prompt: Prompt,
        _cwd: &Path,
    ) -> Result<BoxStream<'static, Result<AgentEvent<Self::Session>, BackendError>>, BackendError>
    {
        // Diagnostics go to stderr so they don't corrupt the ACP stream.
        eprintln!("[zed_agent] prompt received: {}", prompt.text);

        // A real backend would call the model and yield streaming chunks here.
        // This demo answers with a canned, streamed reply to prove the wiring.
        let reply = format!(
            "你好,我是 Longbridge AI(示例 agent)。你刚才说:「{}」。\
             把这里换成真正的模型调用即可。",
            prompt.text
        );

        let events = vec![
            Ok(AgentEvent::Text(reply)),
            Ok(AgentEvent::Finished(())),
        ];
        Ok(Box::pin(stream::iter(events)))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("[zed_agent] starting Longbridge ACP agent on stdio...");
    serve_stdio(MyBackend).await?;
    Ok(())
}
