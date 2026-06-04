use regex::{Regex, RegexBuilder};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseFilterAction {
    Redact,
    Reject,
    RejectAndExpireCredential,
    RejectAndCooldownChannel,
}

impl ResponseFilterAction {
    pub fn is_rejecting(self) -> bool {
        !matches!(self, Self::Redact)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Redact => "redact",
            Self::Reject => "reject",
            Self::RejectAndExpireCredential => "reject_and_expire_credential",
            Self::RejectAndCooldownChannel => "reject_and_cooldown_channel",
        }
    }

    pub fn lifecycle_failure_scope(self) -> Option<crate::error::FailureScope> {
        match self {
            Self::RejectAndExpireCredential => Some(crate::error::FailureScope::Credential),
            Self::RejectAndCooldownChannel => Some(crate::error::FailureScope::Channel),
            Self::Redact | Self::Reject => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponseFilterRuleKind {
    Literal { value: String, case_sensitive: bool },
    Regex { pattern: String },
    RequiredLiteral { value: String, case_sensitive: bool },
    RequiredRegex { pattern: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseFilterRuleSpec {
    pub id: String,
    pub kind: ResponseFilterRuleKind,
    pub action: ResponseFilterAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseFilterSpec {
    pub enabled: bool,
    pub replacement: String,
    pub rules: Vec<ResponseFilterRuleSpec>,
}

#[derive(Debug, Clone)]
pub struct ResponseFilterPolicy {
    enabled: bool,
    replacement: String,
    rules: Vec<CompiledResponseFilterRule>,
}

impl Default for ResponseFilterPolicy {
    fn default() -> Self {
        Self::disabled()
    }
}

#[derive(Debug, Clone)]
struct CompiledResponseFilterRule {
    id: String,
    action: ResponseFilterAction,
    matcher: ResponseFilterMatcher,
}

#[derive(Debug, Clone)]
enum ResponseFilterMatcher {
    Literal {
        value: String,
        normalized_value: String,
        case_sensitive: bool,
    },
    Regex(Regex),
    RequiredLiteral {
        normalized_value: String,
        case_sensitive: bool,
    },
    RequiredRegex(Regex),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponseFilterDecision {
    Unchanged,
    Redacted {
        text: String,
        matches: Vec<ResponseFilterMatch>,
    },
    Rejected {
        matches: Vec<ResponseFilterMatch>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseFilterMatch {
    pub rule_id: String,
    pub action: ResponseFilterAction,
    pub reason_code: &'static str,
}

impl ResponseFilterMatch {
    fn rule_matched(rule_id: String, action: ResponseFilterAction) -> Self {
        Self {
            rule_id,
            action,
            reason_code: "rule_matched",
        }
    }

    fn required_rule_missing(rule_id: String, action: ResponseFilterAction) -> Self {
        Self {
            rule_id,
            action,
            reason_code: "required_rule_missing",
        }
    }
}

impl ResponseFilterPolicy {
    pub fn compile(spec: ResponseFilterSpec) -> anyhow::Result<Self> {
        let replacement = if spec.replacement.is_empty() {
            "[filtered]".to_string()
        } else {
            spec.replacement
        };
        let mut rules = Vec::new();
        for rule in spec.rules {
            let id = rule.id.trim().to_string();
            anyhow::ensure!(!id.is_empty(), "response filter rule id must not be empty");
            let matcher = match rule.kind {
                ResponseFilterRuleKind::Literal {
                    value,
                    case_sensitive,
                } => {
                    anyhow::ensure!(
                        !value.is_empty(),
                        "response filter rule {id} literal must not be empty"
                    );
                    ResponseFilterMatcher::Literal {
                        normalized_value: normalize_filter_text(&value, case_sensitive),
                        value,
                        case_sensitive,
                    }
                }
                ResponseFilterRuleKind::Regex { pattern } => {
                    anyhow::ensure!(
                        !pattern.is_empty(),
                        "response filter rule {id} regex pattern must not be empty"
                    );
                    ResponseFilterMatcher::Regex(
                        RegexBuilder::new(&pattern)
                            .unicode(true)
                            .build()
                            .map_err(|err| {
                                anyhow::anyhow!("invalid response filter regex {id}: {err}")
                            })?,
                    )
                }
                ResponseFilterRuleKind::RequiredLiteral {
                    value,
                    case_sensitive,
                } => {
                    anyhow::ensure!(
                        !value.is_empty(),
                        "response filter rule {id} required literal must not be empty"
                    );
                    ResponseFilterMatcher::RequiredLiteral {
                        normalized_value: normalize_filter_text(&value, case_sensitive),
                        case_sensitive,
                    }
                }
                ResponseFilterRuleKind::RequiredRegex { pattern } => {
                    anyhow::ensure!(
                        !pattern.is_empty(),
                        "response filter rule {id} required regex pattern must not be empty"
                    );
                    ResponseFilterMatcher::RequiredRegex(
                        RegexBuilder::new(&pattern)
                            .unicode(true)
                            .build()
                            .map_err(|err| {
                                anyhow::anyhow!(
                                    "invalid response filter required regex {id}: {err}"
                                )
                            })?,
                    )
                }
            };
            rules.push(CompiledResponseFilterRule {
                id,
                action: rule.action,
                matcher,
            });
        }
        Ok(Self {
            enabled: spec.enabled,
            replacement,
            rules,
        })
    }

    pub fn disabled() -> Self {
        Self {
            enabled: false,
            replacement: "[filtered]".to_string(),
            rules: Vec::new(),
        }
    }

    pub fn inspect_text(&self, text: &str) -> ResponseFilterDecision {
        self.inspect_text_with_required_missing(text, true)
    }

    pub fn inspect_text_for_precommit(
        &self,
        text: &str,
        allow_required_missing: bool,
    ) -> ResponseFilterDecision {
        self.inspect_text_with_required_missing(text, allow_required_missing)
    }

    fn inspect_text_with_required_missing(
        &self,
        text: &str,
        allow_required_missing: bool,
    ) -> ResponseFilterDecision {
        if !self.enabled || self.rules.is_empty() || text.is_empty() {
            return ResponseFilterDecision::Unchanged;
        }

        let mut matches = Vec::new();
        let mut redacted_text = text.to_string();
        for rule in &self.rules {
            match &rule.matcher {
                ResponseFilterMatcher::Literal {
                    value,
                    normalized_value,
                    case_sensitive,
                } => {
                    let normalized_text = normalize_filter_text(text, *case_sensitive);
                    if !normalized_text.contains(normalized_value) {
                        continue;
                    }
                    matches.push(ResponseFilterMatch::rule_matched(
                        rule.id.clone(),
                        rule.action,
                    ));
                    if rule.action.is_rejecting() {
                        return ResponseFilterDecision::Rejected { matches };
                    }
                    redacted_text =
                        redact_literal(&redacted_text, value, *case_sensitive, &self.replacement);
                }
                ResponseFilterMatcher::Regex(regex) => {
                    if !regex.is_match(text) {
                        continue;
                    }
                    matches.push(ResponseFilterMatch::rule_matched(
                        rule.id.clone(),
                        rule.action,
                    ));
                    if rule.action.is_rejecting() {
                        return ResponseFilterDecision::Rejected { matches };
                    }
                    redacted_text = regex
                        .replace_all(&redacted_text, self.replacement.as_str())
                        .to_string();
                }
                ResponseFilterMatcher::RequiredLiteral {
                    normalized_value,
                    case_sensitive,
                } => {
                    let normalized_text = normalize_filter_text(text, *case_sensitive);
                    if normalized_text.contains(normalized_value) {
                        continue;
                    }
                    if !allow_required_missing {
                        continue;
                    }
                    matches.push(ResponseFilterMatch::required_rule_missing(
                        rule.id.clone(),
                        rule.action,
                    ));
                    if rule.action.is_rejecting() {
                        return ResponseFilterDecision::Rejected { matches };
                    }
                    redacted_text = self.replacement.clone();
                }
                ResponseFilterMatcher::RequiredRegex(regex) => {
                    if regex.is_match(text) {
                        continue;
                    }
                    if !allow_required_missing {
                        continue;
                    }
                    matches.push(ResponseFilterMatch::required_rule_missing(
                        rule.id.clone(),
                        rule.action,
                    ));
                    if rule.action.is_rejecting() {
                        return ResponseFilterDecision::Rejected { matches };
                    }
                    redacted_text = self.replacement.clone();
                }
            }
        }

        if matches.is_empty() {
            ResponseFilterDecision::Unchanged
        } else {
            ResponseFilterDecision::Redacted {
                text: redacted_text,
                matches,
            }
        }
    }

    pub fn is_effective(&self) -> bool {
        self.enabled && !self.rules.is_empty()
    }
}

fn normalize_filter_text(text: &str, case_sensitive: bool) -> String {
    let without_hidden: String = text
        .chars()
        .filter(|ch| !is_ignored_format_char(*ch))
        .collect();
    if case_sensitive {
        without_hidden
    } else {
        without_hidden.to_lowercase()
    }
}

fn is_ignored_format_char(ch: char) -> bool {
    matches!(ch, '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{feff}')
}

fn redact_literal(text: &str, value: &str, case_sensitive: bool, replacement: &str) -> String {
    let normalized_value = normalize_filter_text(value, case_sensitive);
    let normalized = normalized_text_with_original_ranges(text, case_sensitive);
    let mut result = String::new();
    let mut original_cursor = 0usize;
    let mut normalized_cursor = 0usize;
    while let Some(relative_start) = normalized.text[normalized_cursor..].find(&normalized_value) {
        let normalized_start = normalized_cursor + relative_start;
        let normalized_end = normalized_start + normalized_value.len();
        let Some(original_start) = normalized.original_start_for(normalized_start) else {
            break;
        };
        let Some(original_end) = normalized.original_end_for(normalized_end) else {
            break;
        };
        result.push_str(&text[original_cursor..original_start]);
        result.push_str(replacement);
        original_cursor = original_end;
        normalized_cursor = normalized_end;
    }
    result.push_str(&text[original_cursor..]);
    result
}

struct NormalizedText {
    text: String,
    original_ranges: Vec<(usize, usize)>,
}

impl NormalizedText {
    fn original_start_for(&self, normalized_byte: usize) -> Option<usize> {
        let char_index = self.text[..normalized_byte].chars().count();
        self.original_ranges
            .get(char_index)
            .map(|(start, _)| *start)
    }

    fn original_end_for(&self, normalized_byte: usize) -> Option<usize> {
        let char_index = self.text[..normalized_byte].chars().count();
        if char_index == 0 {
            return Some(0);
        }
        self.original_ranges
            .get(char_index - 1)
            .map(|(_, end)| *end)
    }
}

fn normalized_text_with_original_ranges(text: &str, case_sensitive: bool) -> NormalizedText {
    let mut normalized = String::new();
    let mut original_ranges = Vec::new();
    for (start, ch) in text.char_indices() {
        if is_ignored_format_char(ch) {
            continue;
        }
        let end = start + ch.len_utf8();
        if case_sensitive {
            normalized.push(ch);
            original_ranges.push((start, end));
        } else {
            for lowered in ch.to_lowercase() {
                normalized.push(lowered);
                original_ranges.push((start, end));
            }
        }
    }
    NormalizedText {
        text: normalized,
        original_ranges,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_policy_leaves_text_unchanged() {
        let policy = ResponseFilterPolicy::disabled();

        assert_eq!(
            policy.inspect_text("marker text"),
            ResponseFilterDecision::Unchanged
        );
    }

    #[test]
    fn literal_blacklist_redacts_case_insensitive_match() {
        let policy = ResponseFilterPolicy::compile(ResponseFilterSpec {
            enabled: true,
            replacement: "[removed]".to_string(),
            rules: vec![ResponseFilterRuleSpec {
                id: "marker".to_string(),
                kind: ResponseFilterRuleKind::Literal {
                    value: "unsafe-marker".to_string(),
                    case_sensitive: false,
                },
                action: ResponseFilterAction::Redact,
            }],
        })
        .unwrap();

        assert_eq!(
            policy.inspect_text("Relay UNSAFE-MARKER payload"),
            ResponseFilterDecision::Redacted {
                text: "Relay [removed] payload".to_string(),
                matches: vec![ResponseFilterMatch {
                    rule_id: "marker".to_string(),
                    action: ResponseFilterAction::Redact,
                    reason_code: "rule_matched",
                }],
            }
        );
    }

    #[test]
    fn regex_blacklist_redacts_match() {
        let policy = ResponseFilterPolicy::compile(ResponseFilterSpec {
            enabled: true,
            replacement: "[filtered]".to_string(),
            rules: vec![ResponseFilterRuleSpec {
                id: "marker".to_string(),
                kind: ResponseFilterRuleKind::Regex {
                    pattern: r"(?i)unsafe\s*marker".to_string(),
                },
                action: ResponseFilterAction::Redact,
            }],
        })
        .unwrap();

        assert_eq!(
            policy.inspect_text("hidden UNSAFE   marker in output"),
            ResponseFilterDecision::Redacted {
                text: "hidden [filtered] in output".to_string(),
                matches: vec![ResponseFilterMatch {
                    rule_id: "marker".to_string(),
                    action: ResponseFilterAction::Redact,
                    reason_code: "rule_matched",
                }],
            }
        );
    }

    #[test]
    fn reject_rule_preempts_redaction() {
        let policy = ResponseFilterPolicy::compile(ResponseFilterSpec {
            enabled: true,
            replacement: "[filtered]".to_string(),
            rules: vec![ResponseFilterRuleSpec {
                id: "blocked-marker".to_string(),
                kind: ResponseFilterRuleKind::Literal {
                    value: "blocked-marker".to_string(),
                    case_sensitive: false,
                },
                action: ResponseFilterAction::Reject,
            }],
        })
        .unwrap();

        assert_eq!(
            policy.inspect_text("contains blocked-marker"),
            ResponseFilterDecision::Rejected {
                matches: vec![ResponseFilterMatch {
                    rule_id: "blocked-marker".to_string(),
                    action: ResponseFilterAction::Reject,
                    reason_code: "rule_matched",
                }],
            }
        );
    }

    #[test]
    fn literal_matching_ignores_zero_width_obfuscation() {
        let policy = ResponseFilterPolicy::compile(ResponseFilterSpec {
            enabled: true,
            replacement: "[filtered]".to_string(),
            rules: vec![ResponseFilterRuleSpec {
                id: "marker".to_string(),
                kind: ResponseFilterRuleKind::Literal {
                    value: "marker".to_string(),
                    case_sensitive: false,
                },
                action: ResponseFilterAction::Reject,
            }],
        })
        .unwrap();

        assert_eq!(
            policy.inspect_text("mar\u{200b}ker"),
            ResponseFilterDecision::Rejected {
                matches: vec![ResponseFilterMatch {
                    rule_id: "marker".to_string(),
                    action: ResponseFilterAction::Reject,
                    reason_code: "rule_matched",
                }],
            }
        );
    }

    #[test]
    fn literal_redaction_removes_zero_width_obfuscated_match() {
        let policy = ResponseFilterPolicy::compile(ResponseFilterSpec {
            enabled: true,
            replacement: "[filtered]".to_string(),
            rules: vec![ResponseFilterRuleSpec {
                id: "marker".to_string(),
                kind: ResponseFilterRuleKind::Literal {
                    value: "marker".to_string(),
                    case_sensitive: false,
                },
                action: ResponseFilterAction::Redact,
            }],
        })
        .unwrap();

        assert_eq!(
            policy.inspect_text("relay mar\u{200b}ker payload"),
            ResponseFilterDecision::Redacted {
                text: "relay [filtered] payload".to_string(),
                matches: vec![ResponseFilterMatch {
                    rule_id: "marker".to_string(),
                    action: ResponseFilterAction::Redact,
                    reason_code: "rule_matched",
                }],
            }
        );
    }

    #[test]
    fn required_literal_rejects_text_without_allowlisted_marker() {
        let policy = ResponseFilterPolicy::compile(ResponseFilterSpec {
            enabled: true,
            replacement: "[filtered]".to_string(),
            rules: vec![ResponseFilterRuleSpec {
                id: "required-marker".to_string(),
                kind: ResponseFilterRuleKind::RequiredLiteral {
                    value: "assistant:".to_string(),
                    case_sensitive: false,
                },
                action: ResponseFilterAction::Reject,
            }],
        })
        .unwrap();

        assert_eq!(
            policy.inspect_text("tool: output"),
            ResponseFilterDecision::Rejected {
                matches: vec![ResponseFilterMatch {
                    rule_id: "required-marker".to_string(),
                    action: ResponseFilterAction::Reject,
                    reason_code: "required_rule_missing",
                }],
            }
        );
        assert_eq!(
            policy.inspect_text("Assistant: output"),
            ResponseFilterDecision::Unchanged
        );
    }

    #[test]
    fn required_regex_redacts_text_without_allowlisted_shape() {
        let policy = ResponseFilterPolicy::compile(ResponseFilterSpec {
            enabled: true,
            replacement: "[filtered]".to_string(),
            rules: vec![ResponseFilterRuleSpec {
                id: "required-json-shape".to_string(),
                kind: ResponseFilterRuleKind::RequiredRegex {
                    pattern: r#"^\s*\{"#.to_string(),
                },
                action: ResponseFilterAction::Redact,
            }],
        })
        .unwrap();

        assert_eq!(
            policy.inspect_text("plain output"),
            ResponseFilterDecision::Redacted {
                text: "[filtered]".to_string(),
                matches: vec![ResponseFilterMatch {
                    rule_id: "required-json-shape".to_string(),
                    action: ResponseFilterAction::Redact,
                    reason_code: "required_rule_missing",
                }],
            }
        );
        assert_eq!(
            policy.inspect_text(r#"{"ok":true}"#),
            ResponseFilterDecision::Unchanged
        );
    }
}
