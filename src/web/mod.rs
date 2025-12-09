//! Web server for OAuth verification and admin panel
//!
//! Runs alongside the Discord bot to handle web-based verification flows
//! and provide an admin interface for configuration management.

mod admin;
mod auth;
mod oauth;
mod server;

pub use admin::{admin_router, AdminState};
pub use auth::{create_session_store, SharedSessionStore};
pub use oauth::OAuthState;
pub use server::{start_web_server, WebServerConfig};
