//! Provider-neutral ACP runtime for Longbridge products.
//!
//! The crate deliberately contains no Longbridge API client, endpoint, or
//! credential handling. Each host implements [`AgentBackend`] using its own API
//! and authorization flow, then runs that backend in-process or over stdio.

mod backend;
mod chart_svg;
mod client;
mod desktop;
mod rich_content;
mod server;

pub use agent_client_protocol as acp;
pub use backend::{
    attachment_payload, AgentBackend, AgentEvent, AgentPlanEntry, AgentSessionInfo,
    AgentSessionPage, BackendError, LoadedAgentSession, Prompt, ATTACHMENT_META_KEY,
};
pub use client::{
    with_external_session, with_initialized_session, with_session, AgentHandshake,
    AllowlistedTools, AllowPermissions, ClientDelegate, DenyPermissions, ExternalAgent,
    ExternalAgentConfig, ExternalAgentKind,
};
pub use desktop::{
    DesktopSession, DesktopSessionEvent, DesktopSessionEvents, DesktopSessionHandle,
    SessionControlError,
};
pub use rich_content::{
    charts_from_markdown, normalize_chart_type, supported_chart_types, RichContent,
    RichContentError, RichContentKind, Table, CHART_MIME_TYPE, RICH_CONTENT_NAMESPACE,
    RICH_CONTENT_VERSION, TABLE_MIME_TYPE,
};
pub use server::{
    acp_agent, acp_agent_with_auth_methods, serve_stdio, serve_stdio_with_auth_methods,
};
