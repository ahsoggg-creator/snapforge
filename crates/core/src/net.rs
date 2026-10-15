//! Shared HTTP client setup. TLS goes through Windows' own SChannel (`native-tls`), so there is no bundled C code.

pub fn agent(timeout_secs: u64) -> ureq::Agent {
    let tls = ureq::tls::TlsConfig::builder()
        .provider(ureq::tls::TlsProvider::NativeTls)
        .build();
    ureq::Agent::config_builder()
        .tls_config(tls)
        .timeout_global(Some(std::time::Duration::from_secs(timeout_secs)))
        // error statuses are handled by the caller, which can show the server's own message
        .http_status_as_error(false)
        .build()
        .into()
}

/// Like `agent`, but never follows redirects (used for plugins: a redirect could leave their allowed hosts).
pub fn agent_no_redirects(timeout_secs: u64) -> ureq::Agent {
    let tls = ureq::tls::TlsConfig::builder()
        .provider(ureq::tls::TlsProvider::NativeTls)
        .build();
    ureq::Agent::config_builder()
        .tls_config(tls)
        .timeout_global(Some(std::time::Duration::from_secs(timeout_secs)))
        .http_status_as_error(false)
        .max_redirects(0)
        .build()
        .into()
}

pub fn user_agent() -> String {
    format!("SnapForge/{}", env!("CARGO_PKG_VERSION"))
}
