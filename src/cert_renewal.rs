use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;
use tracing::{info, warn};

#[derive(Deserialize)]
struct ApiResponse {
    status: u8,
    data: Option<CertData>,
    errors: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct CertData {
    pub crt: String,
    pub key: String,
}

pub struct RenewalStatus {
    pub updated: bool,
    pub domain: String,
}

/// Fetches the current certificate from cPanel and writes it to disk if it has changed.
///
/// Skipped silently when CPANEL_* env vars are not set (e.g. local dev).
/// On network or API failure a warning is logged and Ok(None) is returned so
/// startup continues with whatever cert is already on disk.
pub async fn renew_cert_if_needed(cert_path: &Path, key_path: &Path) -> Result<Option<RenewalStatus>> {
    let (api_token, username, hostname, domain) = match cpanel_config() {
        Some(cfg) => cfg,
        None => {
            info!("cPanel env vars not set — skipping certificate renewal");
            return Ok(None);
        }
    };

    info!("Checking certificate renewal from {}...", hostname);

    let data = match fetch_cert(&api_token, &username, &hostname, &domain).await {
        Ok(d) => d,
        Err(e) => {
            warn!("Certificate renewal check failed, continuing with existing cert: {}", e);
            return Ok(None);
        }
    };

    let existing = std::fs::read_to_string(cert_path).unwrap_or_default();
    let updated = existing.trim() != data.crt.trim();

    if updated {
        write_atomic(cert_path, data.crt.as_bytes()).context("Failed to write cert")?;
        write_atomic(key_path, data.key.as_bytes()).context("Failed to write key")?;
        info!("Certificate updated successfully");
    } else {
        info!("Certificate unchanged — no update needed");
    }

    Ok(Some(RenewalStatus { updated, domain }))
}

fn cpanel_config() -> Option<(String, String, String, String)> {
    let token    = std::env::var("CPANEL_API_TOKEN").ok()?;
    let username = std::env::var("CPANEL_USERNAME").ok()?;
    let hostname = std::env::var("CPANEL_HOSTNAME").ok()?;
    let domain   = std::env::var("CPANEL_DOMAIN").ok()?;
    Some((token, username, hostname, domain))
}

async fn fetch_cert(api_token: &str, username: &str, hostname: &str, domain: &str) -> Result<CertData> {
    let url = format!(
        "https://{}:2083/execute/SSL/fetch_best_for_domain?domain={}",
        hostname, domain
    );

    let response: ApiResponse = reqwest::Client::new()
        .get(&url)
        .header("Authorization", format!("cpanel {}:{}", username, api_token))
        .send()
        .await
        .context("Failed to reach cPanel API")?
        .json()
        .await
        .context("Failed to parse cPanel API response")?;

    if response.status != 1 {
        anyhow::bail!("cPanel API error: {:?}", response.errors);
    }

    response.data.context("No data in cPanel API response")
}

fn write_atomic(path: &Path, contents: &[u8]) -> Result<()> {
    let dir = path.parent().context("Invalid cert path")?;
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".{}.tmp", path.file_name().unwrap_or_default().to_string_lossy()));
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}
