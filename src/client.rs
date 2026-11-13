use agent_client_protocol::schema::v1::{
    AgentCapabilities, Implementation, InitializeRequest, PermissionOption, PermissionOptionKind,
    RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
    SelectedPermissionOutcome,
};
use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::{AcpAgent, AcpAgentConfig, ActiveSession, Agent, Client, ConnectTo};
// use agent_client_protocol_schema::v1::{HttpHeader, McpServer, McpServerHttp, NewSessionRequest};
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;

/// Launch configuration for an external ACP agent such as Codex or Claude.
pub type ExternalAgentConfig = AcpAgentConfig;

/// Official ACP subprocess component. The external agent keeps ownership of
/// its provider authentication, model selection, and native configuration.
pub type ExternalAgent = AcpAgent;

/// ACP adapters supported by Longbridge desktop products.
///
/// The native `codex` and `claude` CLIs do not themselves expose ACP. Hosts
/// launch the separately installed adapters and keep provider authentication in
/// those adapters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExternalAgentKind {
    Codex,
    Claude,
}

impl ExternalAgentKind {
    /// Build a launch configuration using the adapter's conventional executable
    /// name. A product may replace this with an absolute, verified path.
    #[must_use]
    pub fn config(self) -> ExternalAgentConfig {
        ExternalAgentConfig::new(match self {
            Self::Codex => "codex-acp",
            Self::Claude => "claude-agent-acp",
        })
    }
}

/// Host callbacks that require desktop UI policy or user interaction.
#[async_trait]
pub trait ClientDelegate: Send + Sync + 'static {
    async fn request_permission(
        &self,
        request: RequestPermissionRequest,
    ) -> RequestPermissionResponse;
}

/// Conservative delegate for read-only/chat-only hosts.
pub struct DenyPermissions;

/// Metadata negotiated before a desktop chat session is created.
///
/// Desktop products can use this value to select UI affordances without
/// depending on provider-specific configuration for Longbridge, Codex, or
/// Claude.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentHandshake {
    pub protocol_version: ProtocolVersion,
    pub capabilities: AgentCapabilities,
    pub implementation: Option<Implementation>,
}

#[async_trait]
impl ClientDelegate for DenyPermissions {
    async fn request_permission(
        &self,
        _request: RequestPermissionRequest,
    ) -> RequestPermissionResponse {
        RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled)
    }
}

/// Permissive delegate for trusted hosts: auto-approves every tool call by
/// selecting an "allow" option offered by the agent (prefers `AllowAlways`,
/// falls back to `AllowOnce`). Returns `Cancelled` only when the agent offers
/// no allow option at all.
pub struct AllowPermissions;

#[async_trait]
impl ClientDelegate for AllowPermissions {
    async fn request_permission(
        &self,
        request: RequestPermissionRequest,
    ) -> RequestPermissionResponse {
        let chosen = request
            .options
            .iter()
            .find(|option| matches!(option.kind, PermissionOptionKind::AllowAlways))
            .or_else(|| {
                request
                    .options
                    .iter()
                    .find(|option| matches!(option.kind, PermissionOptionKind::AllowOnce))
            });

        let outcome = match chosen {
            Some(option) => RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(
                option.option_id.clone(),
            )),
            None => RequestPermissionOutcome::Cancelled,
        };
        RequestPermissionResponse::new(outcome)
    }
}

/// Auto-approves MCP tools and the claude-skills workflow, rejecting every
/// other tool call.
///
/// Allowed, as surfaced in the tool call's `title`:
/// - `mcp__*` — any MCP server tool, named `mcp__<server>__<tool>`.
/// - `Load skill: *` — launching any claude skill.
/// - `algo *` — the `algo` CLI commands a skill runs through Bash.
///
/// Also allowed are file tools (Read/Grep/Glob/Edit/…) whose target lives
/// under a claude skills directory, detected from the tool call's `locations`
/// or `raw_input` (see [`SKILL_PATH_MARKERS`]).
pub struct AllowlistedTools;

/// Substrings that mark a path as belonging to a claude skills directory.
/// Matched against both the canonical `~/.claude/skills/` location and the
/// `~/.agents/skills/` symlink target, so it works regardless of the user.
const SKILL_PATH_MARKERS: [&str; 2] = ["/.claude/skills/", "/.agents/skills/"];

impl AllowlistedTools {
    /// Whether the tool call is on the allowlist: any MCP tool, a skill load,
    /// an `algo` CLI command, or a file tool targeting a claude skills path.
    fn is_allowed(request: &RequestPermissionRequest) -> bool {
        let title_allowed = request
            .tool_call
            .fields
            .title
            .as_deref()
            .is_some_and(|title| {
                title.starts_with("mcp__")
                    || title.starts_with("Load skill: ")
                    || title.starts_with("algo ")
            });
        title_allowed || Self::targets_skill_path(request)
    }

    /// Whether the tool call targets a file under a claude skills directory,
    /// inspecting the reported `locations` and the raw tool input.
    fn targets_skill_path(request: &RequestPermissionRequest) -> bool {
        let fields = &request.tool_call.fields;
        if let Some(locations) = &fields.locations {
            if locations
                .iter()
                .any(|location| Self::is_skill_path(&location.path.to_string_lossy()))
            {
                return true;
            }
        }
        if let Some(raw_input) = &fields.raw_input {
            if Self::is_skill_path(&raw_input.to_string()) {
                return true;
            }
        }
        false
    }

    /// Whether `text` references a claude skills directory.
    fn is_skill_path(text: &str) -> bool {
        SKILL_PATH_MARKERS.iter().any(|marker| text.contains(marker))
    }

    /// Pick the first option of `primary` kind, else the first of `fallback`.
    fn pick(
        options: &[PermissionOption],
        primary: PermissionOptionKind,
        fallback: PermissionOptionKind,
    ) -> Option<&PermissionOption> {
        options
            .iter()
            .find(|option| option.kind == primary)
            .or_else(|| options.iter().find(|option| option.kind == fallback))
    }
}

#[async_trait]
impl ClientDelegate for AllowlistedTools {
    async fn request_permission(
        &self,
        request: RequestPermissionRequest,
    ) -> RequestPermissionResponse {
        // Allowlisted tools get an allow option; anything else gets rejected.
        let (primary, fallback) = if Self::is_allowed(&request) {
            (
                PermissionOptionKind::AllowOnce,
                PermissionOptionKind::AllowAlways,
            )
        } else {
            (
                PermissionOptionKind::RejectAlways,
                PermissionOptionKind::RejectOnce,
            )
        };

        let outcome = match Self::pick(&request.options, primary, fallback) {
            Some(option) => RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(
                option.option_id.clone(),
            )),
            None => RequestPermissionOutcome::Cancelled,
        };
        RequestPermissionResponse::new(outcome)
    }
}

/// Run one persistent ACP session against any agent transport.
///
/// Passing [`crate::acp_agent`] keeps Longbridge AI fully in-process. Passing
/// [`ExternalAgent`] launches Codex, Claude, or another ACP subprocess. The
/// callback receives an [`ActiveSession`] and may send any number of prompts.
pub async fn with_session<R>(
    agent: impl ConnectTo<Client> + 'static,
    cwd: impl Into<PathBuf>,
    delegate: Arc<dyn ClientDelegate>,
    operation: impl for<'runner> AsyncFnOnce(
        ActiveSession<'runner, Agent>,
    ) -> Result<R, agent_client_protocol::Error>,
) -> Result<R, agent_client_protocol::Error> {
    with_initialized_session(agent, cwd, delegate, async move |_handshake, session| {
        operation(session).await
    })
    .await
}

/// Initialize an ACP agent, expose its negotiated capabilities, and run one
/// persistent session.
pub async fn with_initialized_session<R>(
    agent: impl ConnectTo<Client> + 'static,
    cwd: impl Into<PathBuf>,
    delegate: Arc<dyn ClientDelegate>,
    operation: impl for<'runner> AsyncFnOnce(
        AgentHandshake,
        ActiveSession<'runner, Agent>,
    ) -> Result<R, agent_client_protocol::Error>,
) -> Result<R, agent_client_protocol::Error> {
    let delegate_for_permissions = Arc::clone(&delegate);
    Client
        .builder()
        .on_receive_request(
            async move |request: RequestPermissionRequest, responder, _connection| {
                responder.respond(delegate_for_permissions.request_permission(request).await)
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(agent, async move |connection| {
            let response = connection
                .send_request(InitializeRequest::new(ProtocolVersion::V1))
                .block_task()
                .await?;
            let handshake = AgentHandshake {
                protocol_version: response.protocol_version,
                capabilities: response.agent_capabilities,
                implementation: response.agent_info,
            };
            // Build a custom `NewSessionRequest` so the agent is told which MCP
            // servers to connect to for this session. Here we register a single
            // stdio MCP server; add more entries to `mcp_servers` as needed.
            // let new_session_request =
            //     NewSessionRequest::new(cwd.into()).mcp_servers(vec![McpServer::Http(
            //         McpServerHttp::new(
            //             "navlane",
            //             "https://navlane.longbridge-inc.com/api/v1/openapi/mcp",
            //         )
            //         .headers(vec![HttpHeader::new(
            //             "X-User-Token",
            //             "nl_e8366fc2a1874ce1ae9fafc28610963f",
            //         )]),
            //     )]);
            connection
                .build_session(cwd.into())
                .block_task()
                .run_until(async move |session| operation(handshake, session).await)
                .await
        })
        .await
}

/// Launch an external ACP process and run a persistent desktop chat session.
pub async fn with_external_session<R>(
    config: ExternalAgentConfig,
    cwd: impl Into<PathBuf>,
    delegate: Arc<dyn ClientDelegate>,
    operation: impl for<'runner> AsyncFnOnce(
        ActiveSession<'runner, Agent>,
    ) -> Result<R, agent_client_protocol::Error>,
) -> Result<R, agent_client_protocol::Error> {
    with_session(ExternalAgent::new(config), cwd, delegate, operation).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{acp_agent, AgentBackend, AgentEvent, BackendError, Prompt};
    use futures::{stream, stream::BoxStream};
    use std::path::Path;

    struct Echo;

    #[async_trait]
    impl AgentBackend for Echo {
        type Session = ();

        async fn prompt(
            &self,
            _session: (),
            prompt: Prompt,
            _cwd: &Path,
        ) -> Result<BoxStream<'static, Result<AgentEvent<()>, BackendError>>, BackendError>
        {
            Ok(Box::pin(stream::iter([
                Ok(AgentEvent::Text(format!("echo: {}", prompt.text))),
                Ok(AgentEvent::Finished(())),
            ])))
        }
    }

    #[tokio::test]
    async fn desktop_client_runs_embedded_agent_without_subprocess() {
        let answer = with_session(
            acp_agent(Echo),
            "/tmp",
            Arc::new(DenyPermissions),
            async |mut session| {
                session.send_prompt("hello")?;
                session.read_to_string().await
            },
        )
        .await
        .expect("embedded ACP session");

        assert_eq!(answer, "echo: hello");
    }

    #[tokio::test]
    async fn desktop_client_receives_negotiated_agent_metadata() {
        let implementation = with_initialized_session(
            acp_agent(Echo),
            "/tmp",
            Arc::new(DenyPermissions),
            async |handshake, _session| Ok(handshake.implementation),
        )
        .await
        .expect("embedded ACP handshake")
        .expect("agent implementation");

        assert_eq!(implementation.name, "Longbridge AI");
    }

    #[test]
    fn external_agent_presets_use_acp_adapters() {
        assert_eq!(
            ExternalAgentKind::Codex.config().command(),
            Path::new("codex-acp")
        );
        assert_eq!(
            ExternalAgentKind::Claude.config().command(),
            Path::new("claude-agent-acp")
        );
    }
}
