use std::sync::RwLock;

use serde::{Deserialize, Serialize};

/// A single model routing rule.
///
/// B1: resolution is single-pass in strict list order — for each rule,
/// "alias exact match" or "glob pattern match"; the first hit wins.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteRule {
    /// Exact model name to match (mutually exclusive with pattern).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub alias: String,
    /// Glob pattern to match (mutually exclusive with alias).
    /// Supported syntax: `*` (any run), `?` (single char), `{a,b}` alternation.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pattern: String,
    /// Read-only echo of the pattern syntax; always `glob` since B1.
    #[serde(default = "default_pattern_syntax")]
    pub pattern_syntax: String,
    /// Concrete upstream model the request is routed to.
    pub target: String,
    /// Optional default reasoning_effort injected when client omits it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub effort: String,
    /// Human-readable note shown in the panel.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// Background-task downgrading (B2): requests whose model matches a source
/// are silently rerouted to the cheap target, preserving premium quota for
/// foreground traffic.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DowngradeRule {
    /// Model names considered background/cheap-tier sources.
    #[serde(default)]
    pub sources: Vec<String>,
    /// The cheap model to reroute to.
    pub target: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// B2: enable heuristic background detection (signals: tools + tool-role
    /// history, `bg-` user prefix; at least 2 signals required). Default off.
    #[serde(default)]
    pub use_heuristic: bool,
    /// B2: also force reasoning_effort to `low` when downgrading. Default off.
    #[serde(default)]
    pub also_downgrade_effort: bool,
    /// B2: log-only mode — record "would have downgraded" without rewriting
    /// the model. For canary-validating heuristic accuracy. Default off.
    #[serde(default)]
    pub log_only: bool,
}

fn default_true() -> bool {
    true
}

fn default_pattern_syntax() -> String {
    "glob".to_string()
}

/// Router configuration persisted to JSON.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RouterConfig {
    #[serde(default)]
    pub rules: Vec<RouteRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downgrade: Option<DowngradeRule>,
}

/// B1: validate a router config before saving. Returns a human-readable
/// error mentioning the 1-based rule index; the caller must reject the
/// whole config atomically (no partial saves).
pub fn validate_config(cfg: &RouterConfig) -> Result<(), String> {
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for (i, rule) in cfg.rules.iter().enumerate() {
        let n = i + 1;
        if !rule.alias.is_empty() && !rule.pattern.is_empty() {
            return Err(format!("rule {}: alias and pattern are mutually exclusive", n));
        }
        if rule.alias.is_empty() && rule.pattern.is_empty() {
            return Err(format!("rule {}: each rule needs an alias or a pattern", n));
        }
        if rule.target.is_empty() {
            return Err(format!("rule {}: each rule needs a target model", n));
        }
        if !rule.pattern.is_empty() {
            if let Err(reason) = glob::validate(&rule.pattern) {
                return Err(format!(
                    "rule {}: invalid pattern '{}': {}",
                    n, rule.pattern, reason
                ));
            }
        }
        if !rule.alias.is_empty() && !seen.insert(rule.alias.as_str()) {
            return Err(format!("rule {}: duplicate alias '{}'", n, rule.alias));
        }
    }
    Ok(())
}

/// Model router: resolves incoming model names through user rules, then
/// falls back to the built-in tier table.
pub struct ModelRouter {
    cfg: RwLock<RouterConfig>,
}

/// Built-in tier table (unchanged from v2; user rules take precedence).
pub struct BuiltinTier {
    pub name: &'static str,
    pub model: &'static str,
    pub effort: &'static str,
}

pub const BUILTIN_TIERS: &[BuiltinTier] = &[
    BuiltinTier { name: "auto", model: "GLM-5.2", effort: "" },
    BuiltinTier { name: "fast-model", model: "openpangu-2.0-flash", effort: "" },
    BuiltinTier { name: "balanced-model", model: "openpangu-2.0-pro", effort: "" },
    BuiltinTier { name: "primary-model", model: "GLM-5.2", effort: "" },
    BuiltinTier { name: "deep-model", model: "GLM-5.2", effort: "max" },
];

/// Result of a resolution: the target model, optional effort, which layer
/// produced the answer, and the B2 log-only observation flag.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub model: String,
    pub effort: String,
    pub via: ResolutionLayer,
    /// B2: true when log_only mode observed a would-be downgrade.
    pub downgrade_observed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionLayer {
    UserRule,
    Downgrade,
    BuiltinTier,
    Passthrough,
}

impl ResolutionLayer {
    pub fn as_str(self) -> &'static str {
        match self {
            ResolutionLayer::UserRule => "user_rule",
            ResolutionLayer::Downgrade => "downgrade",
            ResolutionLayer::BuiltinTier => "builtin_tier",
            ResolutionLayer::Passthrough => "passthrough",
        }
    }
}

impl ModelRouter {
    pub fn new(cfg: RouterConfig) -> Self {
        Self {
            cfg: RwLock::new(cfg),
        }
    }

    pub fn config(&self) -> RouterConfig {
        match self.cfg.read() {
            Ok(g) => g.clone(),
            Err(_) => {
                tracing::error!("router config lock poisoned (read)");
                RouterConfig::default()
            }
        }
    }

    pub fn replace(&self, cfg: RouterConfig) {
        match self.cfg.write() {
            Ok(mut g) => *g = cfg,
            Err(_) => {
                tracing::error!("router config lock poisoned (write), replace dropped");
            }
        }
    }

    /// Base resolution (user rules -> builtin tiers -> passthrough) without
    /// the background downgrade. B1: single pass, strict list order — the
    /// first rule whose alias equals the model or whose glob pattern matches
    /// wins. B2: allowlist checks and audit use this pre-downgrade result.
    pub fn resolve_base(&self, model: &str, effort: &str) -> Resolved {
        let cfg = match self.cfg.read() {
            Ok(g) => g,
            Err(_) => {
                tracing::error!("router config lock poisoned (read), passthrough");
                return Resolved {
                    model: model.to_string(),
                    effort: effort.to_string(),
                    via: ResolutionLayer::Passthrough,
                    downgrade_observed: false,
                };
            }
        };

        // 1. User rules: alias exact match or glob pattern match, in order.
        for rule in &cfg.rules {
            let hit = if !rule.alias.is_empty() {
                rule.alias == model
            } else if !rule.pattern.is_empty() {
                glob::matches(&rule.pattern, model)
            } else {
                false
            };
            if hit {
                return Resolved {
                    model: rule.target.clone(),
                    effort: if effort.is_empty() {
                        rule.effort.clone()
                    } else {
                        effort.to_string()
                    },
                    via: ResolutionLayer::UserRule,
                    downgrade_observed: false,
                };
            }
        }

        // 2. Built-in tiers.
        if let Some(t) = BUILTIN_TIERS.iter().find(|t| t.name == model) {
            return Resolved {
                model: t.model.to_string(),
                effort: if effort.is_empty() {
                    t.effort.to_string()
                } else {
                    effort.to_string()
                },
                via: ResolutionLayer::BuiltinTier,
                downgrade_observed: false,
            };
        }

        // 3. Passthrough.
        Resolved {
            model: model.to_string(),
            effort: effort.to_string(),
            via: ResolutionLayer::Passthrough,
            downgrade_observed: false,
        }
    }

    /// B2: apply the background downgrade on top of a base resolution.
    /// `log_only` keeps the model but marks the entry as observed so the
    /// audit log can show "would have downgraded".
    pub fn apply_downgrade(&self, base: Resolved, is_background: bool) -> Resolved {
        if !is_background {
            return base;
        }
        let cfg = match self.cfg.read() {
            Ok(g) => g,
            Err(_) => {
                tracing::error!("router config lock poisoned (read), downgrade skipped");
                return base;
            }
        };
        if let Some(d) = &cfg.downgrade {
            if d.enabled && d.sources.iter().any(|s| s == &base.model) {
                if d.log_only {
                    return Resolved {
                        model: base.model,
                        effort: base.effort,
                        via: ResolutionLayer::Downgrade,
                        downgrade_observed: true,
                    };
                }
                return Resolved {
                    model: d.target.clone(),
                    effort: if d.also_downgrade_effort {
                        "low".to_string()
                    } else {
                        base.effort
                    },
                    via: ResolutionLayer::Downgrade,
                    downgrade_observed: false,
                };
            }
        }
        base
    }

    /// Public model ids exposed via /v1/models: user aliases + builtin tiers.
    pub fn advertised_aliases(&self) -> Vec<(String, String)> {
        let cfg = match self.cfg.read() {
            Ok(g) => g,
            Err(_) => {
                tracing::error!("router config lock poisoned (read)");
                return Vec::new();
            }
        };
        cfg.rules
            .iter()
            .filter(|r| !r.alias.is_empty())
            .map(|r| (r.alias.clone(), r.note.clone()))
            .collect()
    }
}

/// B1: glob pattern engine. Supported syntax: `*` (any run of chars),
/// `?` (exactly one char), `{a,b,...}` (alternation). Any other regex-style
/// metacharacter is rejected at validation time — no silent fallback to
/// substring matching.
mod glob {
    /// Validate a pattern; Err carries a human-readable reason.
    pub fn validate(pattern: &str) -> Result<(), String> {
        if pattern.is_empty() {
            return Err("pattern is empty".to_string());
        }
        let mut depth = 0usize;
        for c in pattern.chars() {
            match c {
                '{' => {
                    depth += 1;
                    if depth > 1 {
                        return Err("nested '{...}' alternation is not supported".to_string());
                    }
                }
                '}' => {
                    if depth == 0 {
                        return Err("unmatched '}'".to_string());
                    }
                    depth -= 1;
                }
                '*' | '?' | ',' => {}
                '|' | '^' | '$' | '.' | '[' | ']' | '(' | ')' | '+' | '\' => {
                    return Err(format!(
                        "metacharacter '{}' is not supported (allowed: * ? {{a,b}})",
                        c
                    ));
                }
                _ => {}
            }
        }
        if depth != 0 {
            return Err("unmatched '{'".to_string());
        }
        Ok(())
    }

    /// Match `text` against a glob `pattern`.
    pub fn matches(pattern: &str, text: &str) -> bool {
        for variant in expand(pattern) {
            let p: Vec<char> = variant.chars().collect();
            let t: Vec<char> = text.chars().collect();
            if match_here(&p, &t) {
                return true;
            }
        }
        false
    }

    /// Expand `{a,b}` alternations into concrete variants.
    fn expand(pattern: &str) -> Vec<String> {
        if let Some(start) = pattern.find('{') {
            if let Some(rel) = pattern[start..].find('}') {
                let end = start + rel;
                let prefix = &pattern[..start];
                let body = &pattern[start + 1..end];
                let suffix = &pattern[end + 1..];
                let mut out = Vec::new();
                for alt in body.split(',') {
                    for s in expand(&format!("{}{}{}", prefix, alt, suffix)) {
                        out.push(s);
                    }
                }
                return out;
            }
        }
        vec![pattern.to_string()]
    }

    fn match_here(pat: &[char], txt: &[char]) -> bool {
        if pat.is_empty() {
            return txt.is_empty();
        }
        match pat[0] {
            '*' => {
                for skip in 0..=txt.len() {
                    if match_here(&pat[1..], &txt[skip..]) {
                        return true;
                    }
                }
                false
            }
            '?' => !txt.is_empty() && match_here(&pat[1..], &txt[1..]),
            c => !txt.is_empty() && txt[0] == c && match_here(&pat[1..], &txt[1..]),
        }
    }
}
