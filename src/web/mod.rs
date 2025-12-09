//! Web server for OAuth verification
//!
//! Runs alongside the Discord bot to handle web-based verification flows.

mod oauth;
mod server;

pub use server::{start_web_server, WebServerConfig};
pub use oauth::OAuthState;
