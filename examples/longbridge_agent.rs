//! A **real** Longbridge ACP *agent* that Zed (or any ACP client) can connect to.
//!
//! Unlike `zed_agent.rs` (which returns a canned string), this one calls the
//! Longbridge internal LLM service `lb.algo.chat.proxy` (`AlgoChatProxy.
//! ChatCompletion`) through the inner-next RPC gateway, and streams the answer
//! back to the client as ACP text. The ACP wiring itself is unchanged — only
//! the body of `prompt()` does real work now.
//!
//! Build it, then point Zed's `agent_servers` at the compiled binary:
//!
//!   cargo build --release --example longbridge_agent
//!   # binary at: target/release/examples/longbridge_agent
//!
//! settings.json (~/.config/zed/settings.json):
//!   {
//!     "agent_servers": {
//!       "Longbridge AI": {
//!         "command": "/ABS/PATH/target/release/examples/longbridge_agent",
//!         "args": [],
//!         "env": {
//!           "LB_GATEWAY_URL": "https://inner-next.longbridge.xyz/rpc",
//!           "LB_RPC_AUTH": "<x-authorization credential for that env>",
//!           "LB_MODEL": "gemini-3-flash-preview"
//!         }
//!       }
//!     }
//!   }
//!
//! Config comes from the environment so no endpoint or secret is baked into the
//! binary, and the target environment stays explicit (never defaults to prod):
//!   LB_GATEWAY_URL  inner-next gateway URL, e.g. staging-HK
//!                   https://inner-next.longbridge.xyz/rpc
//!   LB_RPC_AUTH     the `x-authorization` credential for THAT environment
//!   LB_MODEL        model name (optional; defaults to gemini-3-flash-preview)
//!   LB_USER_REGION  value for the `user-region` header (optional; default HK)
//!
//! IMPORTANT: stdout is the ACP protocol channel. Never `println!` here — all
//! diagnostics must go to stderr (`eprintln!`).
use async_trait::async_trait;
use futures::stream::{self, BoxStream};
use longbridge_ai_acp::{serve_stdio, AgentBackend, AgentEvent, BackendError, Prompt};
use serde_json::{json, Value};
use std::path::Path;

/// Holds the HTTP client and the gateway coordinates read from the environment.
struct LongbridgeBackend {
    http: reqwest::Client,
    gateway_url: String,
    auth: String,
    model: String,
    user_region: String,
}

impl LongbridgeBackend {
    /// Builds the backend from environment variables, failing fast if the two
    /// required ones (gateway URL + credential) are missing.
    fn from_env() -> Result<Self, String> {
        let gateway_url = std::env::var("LB_GATEWAY_URL").map_err(|_| {
            "LB_GATEWAY_URL is not set (e.g. https://inner-next.longbridge.xyz/rpc)".to_string()
        })?;
        let auth = std::env::var("LB_RPC_AUTH")
            .map_err(|_| "LB_RPC_AUTH is not set (the x-authorization credential)".to_string())?;
        let model =
            std::env::var("LB_MODEL").unwrap_or_else(|_| "gemini-3-flash-preview".to_string());
        let user_region = std::env::var("LB_USER_REGION").unwrap_or_else(|_| "HK".to_string());
        Ok(Self {
            http: reqwest::Client::new(),
            gateway_url,
            auth,
            model,
            user_region,
        })
    }

    /// Sends one user turn to `AlgoChatProxy.ChatCompletion` and returns the
    /// assistant text. Maps the inner-next RPC envelope onto the proto's
    /// `ChatReq` / `ChatReply` messages.
    async fn chat(&self, user_text: &str) -> Result<String, String> {
        // RPC envelope understood by the inner-next gateway, wrapping a `ChatReq`.
        let body = json!({
            "service": "lb.algo.chat.proxy",
            "method": "AlgoChatProxy.ChatCompletion",
            "request": {
                "model": self.model,
                "messages": [{ "role": "user", "content": user_text }],
                "from_service": "longbridge-ai-acp-example",
            },
        });

        let resp = self
            .http
            .post(&self.gateway_url)
            .header("x-authorization", &self.auth)
            .header("x-code", "37f3821e")
            .header("app-id", "longbridge")
            .header("accept-language", "zh-CN")
            .header("user-region", &self.user_region)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("gateway request failed: {e}"))?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| format!("reading gateway body failed: {e}"))?;
        if !status.is_success() {
            return Err(format!("gateway returned HTTP {status}: {text}"));
        }

        let value: Value =
            serde_json::from_str(&text).map_err(|e| format!("gateway body was not JSON: {e}; raw={text}"))?;

        // `ChatReply.chat_reply` is the ready-made answer; fall back to the
        // OpenAI-style `choices[0].message.content` if it is empty.
        if let Some(reply) = value.get("chat_reply").and_then(Value::as_str) {
            if !reply.is_empty() {
                return Ok(reply.to_string());
            }
        }
        if let Some(content) = value
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str)
        {
            if !content.is_empty() {
                return Ok(content.to_string());
            }
        }
        Err(format!("no assistant text in gateway reply: {text}"))
    }
}

#[async_trait]
impl AgentBackend for LongbridgeBackend {
    type Session = ();

    async fn prompt(
        &self,
        _session: Self::Session,
        prompt: Prompt,
        _cwd: &Path,
    ) -> Result<BoxStream<'static, Result<AgentEvent<Self::Session>, BackendError>>, BackendError>
    {
        // Diagnostics go to stderr so they don't corrupt the ACP stream.
        eprintln!("[longbridge_agent] prompt received: {}", prompt.text);

        // Call the real model. On failure, surface the error to the client as
        // text (and log it) instead of tearing down the ACP turn, so the user
        // sees what went wrong inside Zed.
        let events = match self.chat(&prompt.text).await {
            Ok(reply) => {
                vec![Ok(AgentEvent::Text(reply)), Ok(AgentEvent::Finished(()))]
            }
            Err(err) => {
                eprintln!("[longbridge_agent] LLM call failed: {err}");
                vec![
                    Ok(AgentEvent::Text(format!("调用 Longbridge AI 失败:{err}"))),
                    Ok(AgentEvent::Finished(())),
                ]
            }
        };
        Ok(Box::pin(stream::iter(events)))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("[longbridge_agent] starting Longbridge ACP agent on stdio...");
    let backend = LongbridgeBackend::from_env().map_err(|e| {
        eprintln!("[longbridge_agent] config error: {e}");
        e
    })?;
    serve_stdio(backend).await?;
    Ok(())
}
