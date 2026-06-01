use serde::Deserialize;
use std::sync::OnceLock;

static FIXTURES: OnceLock<TestFixtures> = OnceLock::new();

#[derive(Debug, Deserialize)]
pub struct TestFixtures {
    pub client_token: String,
    pub admin_token: String,
    pub restricted_client_token: String,
    pub disabled_client_token: String,
    pub created_client_token: String,
    pub temporary_client_token: String,
    pub scoped_client_token: String,
    pub persisted_client_token: String,
    pub persisted_scope_client_token: String,
    pub db_only_client_token: String,
    pub scoped_runtime_token: String,
    pub upstream_credentials: UpstreamCredentials,
    pub redaction_samples: RedactionSamples,
}

#[derive(Debug, Deserialize)]
pub struct UpstreamCredentials {
    pub a: String,
    pub b: String,
    pub c: String,
    pub manual: String,
    pub legacy: String,
    pub test_upstream: String,
    pub hash_sample: String,
    pub one: String,
    pub two: String,
    pub same: String,
    pub debug_client: String,
    pub debug_admin: String,
    pub local_router: String,
    pub local_admin: String,
}

#[derive(Debug, Deserialize)]
pub struct RedactionSamples {
    pub sensitive_token: String,
    pub spaced_secret: String,
    pub colon_secret: String,
    pub whitespace_secret: String,
    pub local_path: String,
    pub github_token: String,
    pub compact_token: String,
    pub basic_secret: String,
    pub cloud_secret: String,
}

pub fn fixtures() -> &'static TestFixtures {
    FIXTURES.get_or_init(|| {
        serde_yaml::from_str(include_str!("../tests/fixtures/test-values.yaml"))
            .expect("test fixture values must be valid YAML")
    })
}

pub fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}

pub fn credential_lines(values: &[&str]) -> String {
    values
        .iter()
        .map(|value| format!("    - {value}\n"))
        .collect()
}

pub fn compact_yaml(raw: String) -> String {
    let lines = raw.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return "\n".to_string();
    }

    let first = lines
        .iter()
        .position(|line| !line.trim().is_empty())
        .unwrap_or(0);
    let last = lines
        .iter()
        .rposition(|line| !line.trim().is_empty())
        .unwrap_or(first);
    let trimmed = &lines[first..=last];
    let indent = trimmed
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start().len())
        .min()
        .unwrap_or(0);

    let mut compact = trimmed
        .iter()
        .map(|line| {
            if line.len() >= indent {
                &line[indent..]
            } else {
                line.trim_start()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    compact.push('\n');
    compact
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fixture_values() {
        let values = fixtures();

        assert_eq!(values.client_token, "fixture-client-token");
        assert_eq!(
            values.upstream_credentials.hash_sample,
            "fixture-upstream-credential-hash-sample"
        );
        assert_eq!(
            values.redaction_samples.github_token,
            "github-fixture-token-unusable"
        );
    }

    #[test]
    fn formats_bearer_credentials_and_yaml() {
        assert_eq!(bearer("fixture-token"), "Bearer fixture-token");
        assert_eq!(
            credential_lines(&["fixture-a", "fixture-b"]),
            "    - fixture-a\n    - fixture-b\n"
        );
        assert_eq!(
            compact_yaml(
                r#"
                    root:
                      child: fixture
                "#
                .to_string(),
            ),
            "root:\n  child: fixture\n"
        );
    }
}
