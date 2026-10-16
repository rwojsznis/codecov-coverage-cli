//! The two Codecov API v2 endpoints the CLI reads: one file's report, and the
//! report for every file of a commit.
//!
//! ref: <https://docs.codecov.com/reference/repos_file_report_retrieve>
//! ref: <https://docs.codecov.com/reference/repos_report_retrieve>

use serde_json::Value;
use std::fmt;
use std::time::Duration;

/// Codecov's hosted API. Self-hosted installations override it.
pub const DEFAULT_API_URL: &str = "https://api.codecov.io";

/// A whole-repository report carries every line of every file, so the default
/// 10 MB cap of `ureq` is too small for a large monorepo.
const BODY_LIMIT: u64 = 1024 * 1024 * 1024;

/// The repository on the Git hosting service, as Codecov names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    /// `github`, `gitlab`, `bitbucket`, or their `_enterprise` / `_server` forms.
    pub service: String,
    pub owner: String,
    pub name: String,
}

/// Which commit a report is for. When neither is set, Codecov uses the head
/// of the repository's default branch.
#[derive(Debug, Clone, Default)]
pub struct Target {
    pub branch: Option<String>,
    pub sha: Option<String>,
}

#[derive(Debug)]
pub struct ApiError {
    /// The HTTP status, when the server answered. `None` for a network or a
    /// decoding failure.
    pub status: Option<u16>,
    pub message: String,
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ApiError {}

#[derive(Debug)]
pub struct Client {
    agent: ureq::Agent,
    /// `<api>/api/v2/<service>/<owner>/repos/<repo>`, already encoded.
    base: String,
    token: String,
}

impl Client {
    pub fn new(api_url: &str, token: &str, repo: &Repo) -> Client {
        let config = ureq::Agent::config_builder()
            // A 404 is an answer the CLI explains, not a transport failure.
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(30)))
            .timeout_global(Some(Duration::from_secs(300)))
            .user_agent(concat!(
                env!("CARGO_PKG_NAME"),
                "/",
                env!("CARGO_PKG_VERSION")
            ))
            .build();
        let base = format!(
            "{}/api/v2/{}/{}/repos/{}",
            api_url.trim_end_matches('/'),
            encode_component(&repo.service),
            encode_component(&repo.owner),
            encode_component(&repo.name),
        );
        Client {
            agent: ureq::Agent::new_with_config(config),
            base,
            token: token.to_owned(),
        }
    }

    /// One file's report: `name`, `totals` and `line_coverage`.
    ///
    /// # Errors
    ///
    /// A transport failure, a non-2xx answer, or a body that is not JSON.
    pub fn file_report(&self, path: &str, target: &Target) -> Result<Value, ApiError> {
        self.get(
            &format!("file_report/{}", encode_component(path)),
            target,
            None,
        )
    }

    /// Every file of one commit, optionally only those whose path starts with
    /// `prefix`: `totals` and `files`, each file shaped like a file report.
    ///
    /// # Errors
    ///
    /// A transport failure, a non-2xx answer, or a body that is not JSON.
    pub fn report(&self, prefix: Option<&str>, target: &Target) -> Result<Value, ApiError> {
        self.get("report/", target, prefix)
    }

    fn get(
        &self,
        endpoint: &str,
        target: &Target,
        prefix: Option<&str>,
    ) -> Result<Value, ApiError> {
        let mut request = self
            .agent
            .get(format!("{}/{endpoint}", self.base))
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Accept", "application/json");
        let params = [
            ("branch", target.branch.as_deref()),
            ("sha", target.sha.as_deref()),
            ("path", prefix),
        ];
        for (key, value) in params {
            if let Some(value) = value.filter(|v| !v.is_empty()) {
                request = request.query(key, value);
            }
        }

        let transport = |e: ureq::Error| ApiError {
            status: None,
            message: e.to_string(),
        };
        let mut response = request.call().map_err(transport)?;
        let status = response.status();
        let body = response
            .body_mut()
            .with_config()
            .limit(BODY_LIMIT)
            .read_to_string()
            .map_err(transport)?;

        if !status.is_success() {
            return Err(ApiError {
                status: Some(status.as_u16()),
                message: format!(
                    "Error fetching coverage report: {} {}\n{body}",
                    status.as_u16(),
                    status.canonical_reason().unwrap_or_default()
                ),
            });
        }

        let mut value: Value = serde_json::from_str(&body).map_err(|e| ApiError {
            status: None,
            message: format!("Codecov answered with a body that is not JSON: {e}"),
        })?;
        // A link and a SHA per response add nothing an agent can act on.
        if let Some(object) = value.as_object_mut() {
            object.remove("commit_file_url");
            object.remove("commit_sha");
        }
        Ok(value)
    }
}

/// Percent-encodes everything but the characters JavaScript's
/// `encodeURIComponent` keeps, so a file path becomes one URL segment
/// (`src/a.rs` → `src%2Fa.rs`), as the Codecov API expects.
pub fn encode_component(text: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 0x0F)]));
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_component_matches_encode_uri_component() {
        assert_eq!(encode_component("src/a.rs"), "src%2Fa.rs");
        assert_eq!(encode_component("a b+c&d"), "a%20b%2Bc%26d");
        assert_eq!(encode_component("keep-_.!~*'()"), "keep-_.!~*'()");
        assert_eq!(encode_component("café"), "caf%C3%A9");
    }

    #[test]
    fn the_base_url_encodes_each_segment_and_drops_a_trailing_slash() {
        let repo = Repo {
            service: "github".into(),
            owner: "acme corp".into(),
            name: "shop".into(),
        };
        let client = Client::new("http://localhost:1/", "t", &repo);
        assert_eq!(
            client.base,
            "http://localhost:1/api/v2/github/acme%20corp/repos/shop"
        );
    }
}
