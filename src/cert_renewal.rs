use anyhow::{Context, Result};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
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
    #[serde(default)]
    pub cabundle: String,
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
pub async fn renew_cert_if_needed(
    cert_path: &Path,
    key_path: &Path,
) -> Result<Option<RenewalStatus>> {
    // Always ensure the on-disk cert has a complete chain, regardless of whether
    // cPanel renewal runs. This is a no-op if the chain is already complete.
    if let Err(e) = ensure_chain_complete(cert_path).await {
        warn!("Could not complete certificate chain: {}", e);
    }

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
            warn!(
                "Certificate renewal check failed, continuing with existing cert: {}",
                e
            );
            return Ok(None);
        }
    };

    // Full chain = leaf cert + intermediate CA bundle.
    let fullchain = if data.cabundle.trim().is_empty() {
        data.crt.clone()
    } else {
        format!("{}\n{}", data.crt.trim(), data.cabundle.trim())
    };

    // Compare only against the leaf cert so a previously chain-less file gets
    // rewritten even if the leaf cert hasn't changed.
    let existing_leaf = first_pem_cert(
        &std::fs::read_to_string(cert_path).unwrap_or_default()
    );
    let updated = existing_leaf.trim() != data.crt.trim();

    if updated {
        write_atomic(cert_path, fullchain.as_bytes()).context("Failed to write cert")?;
        write_atomic(key_path, data.key.as_bytes()).context("Failed to write key")?;
        info!("Certificate updated successfully");
    } else {
        info!("Certificate unchanged — no update needed");
    }

    Ok(Some(RenewalStatus { updated, domain }))
}

/// Ensures cert_path contains a full certificate chain (leaf + intermediates).
///
/// If only the leaf is present, the issuer certificate is fetched from the
/// CA Issuers URL embedded in the cert's AIA extension and appended. This
/// runs on every startup so no manual intervention is ever needed.
async fn ensure_chain_complete(cert_path: &Path) -> Result<()> {
    let pem = match std::fs::read_to_string(cert_path) {
        Ok(p) => p,
        Err(_) => return Ok(()), // no cert yet, nothing to complete
    };

    if pem.matches("-----BEGIN CERTIFICATE-----").count() > 1 {
        return Ok(()); // chain already complete
    }

    let der = pem_to_der(&pem).context("Failed to decode leaf certificate")?;

    let issuer_url = match find_aia_issuer_url(&der) {
        Some(url) => url,
        None => {
            warn!("No CA Issuers URL found in certificate AIA extension");
            return Ok(());
        }
    };

    info!("Fetching intermediate certificate from {}", issuer_url);

    let bytes = reqwest::get(&issuer_url)
        .await
        .context("Failed to fetch intermediate cert")?
        .bytes()
        .await
        .context("Failed to read intermediate cert response")?;

    // The response may be DER or PEM — detect by checking for PEM header.
    let intermediate_pem = if bytes.starts_with(b"-----BEGIN") {
        String::from_utf8_lossy(&bytes).into_owned()
    } else {
        der_bytes_to_pem(&bytes)
    };

    let fullchain = format!("{}\n{}", pem.trim(), intermediate_pem.trim());
    write_atomic(cert_path, fullchain.as_bytes()).context("Failed to write full chain")?;
    info!("Certificate chain completed successfully");

    Ok(())
}

/// Extract the first PEM certificate block from a PEM string.
fn first_pem_cert(pem: &str) -> String {
    let start = "-----BEGIN CERTIFICATE-----";
    let end = "-----END CERTIFICATE-----";
    if let Some(s) = pem.find(start) {
        if let Some(e) = pem[s..].find(end) {
            return pem[s..s + e + end.len()].to_string();
        }
    }
    pem.to_string()
}

/// Decode the first PEM certificate to DER bytes.
fn pem_to_der(pem: &str) -> Result<Vec<u8>> {
    let start = "-----BEGIN CERTIFICATE-----";
    let end = "-----END CERTIFICATE-----";
    let s = pem.find(start).context("No BEGIN CERTIFICATE marker")?;
    let e = pem[s..].find(end).context("No END CERTIFICATE marker")?;
    let b64: String = pem[s + start.len()..s + e]
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    BASE64.decode(b64).context("Failed to base64-decode certificate")
}

/// Encode DER bytes as a PEM certificate string.
fn der_bytes_to_pem(der: &[u8]) -> String {
    let b64 = BASE64.encode(der);
    let body: String = b64
        .chars()
        .collect::<Vec<_>>()
        .chunks(64)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("\n");
    format!("-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----\n", body)
}

/// Find the CA Issuers URL in the AIA extension of a DER-encoded certificate.
///
/// The URL appears as raw ASCII bytes in the DER, so we can find it by scanning
/// for "http://" without needing a full ASN.1 parser.
fn find_aia_issuer_url(der: &[u8]) -> Option<String> {
    let marker = b"http://";
    let mut i = 0;
    while i + marker.len() <= der.len() {
        if &der[i..i + marker.len()] == marker {
            let end = der[i..]
                .iter()
                .position(|&b| !b.is_ascii_graphic() || b == b'"')
                .unwrap_or(der.len() - i);
            let url = std::str::from_utf8(&der[i..i + end]).ok()?;
            // The AIA CA Issuers URL typically points to a .crt or .cer file.
            if url.ends_with(".crt") || url.ends_with(".cer") || url.contains(".i.lencr.org") {
                return Some(url.to_string());
            }
        }
        i += 1;
    }
    None
}

fn cpanel_config() -> Option<(String, String, String, String)> {
    let token = std::env::var("CPANEL_API_TOKEN").ok()?;
    let username = std::env::var("CPANEL_USERNAME").ok()?;
    let hostname = std::env::var("CPANEL_HOSTNAME").ok()?;
    let domain = std::env::var("CPANEL_DOMAIN").ok()?;
    Some((token, username, hostname, domain))
}

async fn fetch_cert(
    api_token: &str,
    username: &str,
    hostname: &str,
    domain: &str,
) -> Result<CertData> {
    let url = format!(
        "https://{}:2083/execute/SSL/fetch_best_for_domain?domain={}",
        hostname, domain
    );

    let response: ApiResponse = reqwest::Client::new()
        .get(&url)
        .header(
            "Authorization",
            format!("cpanel {}:{}", username, api_token),
        )
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
    let tmp = dir.join(format!(
        ".{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy()
    ));
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}
