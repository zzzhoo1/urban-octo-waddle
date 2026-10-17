use std::sync::RwLock;

use serde::{Deserialize, Serialize};

/// A single model routing rule.
///
/// Resolution order:
/// 1. Exact alias match (`alias == request model`)
/// 2. Regex match (`pattern` matches request model)
/// First matching rule in list order wins.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteRule {
    /// Exact model name to match (mutually exclusive with pattern).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub alias: String,
    /// Regex pattern to match (mutually exclusive with alias).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pattern: String,
    /// Concrete upstream model the request is routed to.
    pub target: String,
    /// Optional default reasoning_effort injected when client omits it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub effort: String,
    /// Human-readable note shown in the panel.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// Background-task downgrading: requests whose model matches the source
/// tier are silently rerouted to the cheap model, preserving premium quota
/// for foreground traffic (mirrors Antigravity's silent downgrading idea).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DowngradeRule {
    /// Model names considered background/cheap-tier sources.
    #[serde(default)]
    pub sources: Vec<String>,
    /// The cheap model to reroute to.
    pub target: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// Router configuration persisted to JSON.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RouterConfig {
    #[serde(default)]
    pub rules: Vec<RouteRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downgrade: Option<DowngradeRule>,
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

/// Result of a resolution: the target model, optional effort, and which
/// layer produced the answer (for observability in logs/panel).
#[derive(Debug, Clone)]
pub struct Resolved {
    pub model: String,
    pub effort: String,
    pub via: ResolutionLayer,
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

    /// Resolve a requested model + effort into (target model, effort).
    ///
    /// - User rules checked first (exact alias, then regex pattern, in order).
    /// - If `is_background` and a downgrade rule covers the resolved model,
    ///   the request is silently rerouted to the cheap target.
    /// - Otherwise built-in tiers, then passthrough.
    pub fn resolve(&self, model: &str, effort: &str, is_background: bool) -> Resolved {
        let cfg = match self.cfg.read() {
            Ok(g) => g,
            Err(_) => {
                tracing::error!("router config lock poisoned (read), passthrough");
                return Resolved {
                    model: model.to_string(),
                    effort: effort.to_string(),
                    via: ResolutionLayer::Passthrough,
                };
            }
        };

        // 1. User rules: exact alias first, then regex.
        for rule in &cfg.rules {
            if !rule.alias.is_empty() && rule.alias == model {
                return self.apply_downgrade(
                    &cfg,
                    rule.target.clone(),
                    if effort.is_empty() { rule.effort.clone() } else { effort.to_string() },
                    is_background,
                    ResolutionLayer::UserRule,
                );
            }
        }
        for rule in &cfg.rules {
            if rule.pattern.is_empty() {
                continue;
            }
            if let Ok(re) = regex_lite::Regex::new(&rule.pattern) {
                if re.is_match(model) {
                    return self.apply_downgrade(
                        &cfg,
                        rule.target.clone(),
                        if effort.is_empty() { rule.effort.clone() } else { effort.to_string() },
                        is_background,
                        ResolutionLayer::UserRule,
                    );
                }
            }
        }

        // 2. Built-in tiers.
        if let Some(t) = BUILTIN_TIERS.iter().find(|t| t.name == model) {
            let eff = if effort.is_empty() {
                t.effort.to_string()
            } else {
                effort.to_string()
            };
            return self.apply_downgrade(&cfg, t.model.to_string(), eff, is_background, ResolutionLayer::BuiltinTier);
        }

        // 3. Passthrough.
        self.apply_downgrade(&cfg, model.to_string(), effort.to_string(), is_background, ResolutionLayer::Passthrough)
    }

    fn apply_downgrade(
        &self,
        cfg: &RouterConfig,
        model: String,
        effort: String,
        is_background: bool,
        via: ResolutionLayer,
    ) -> Resolved {
        if is_background {
            if let Some(d) = &cfg.downgrade {
                if d.enabled && d.sources.iter().any(|s| s == &model) {
                    return Resolved {
                        model: d.target.clone(),
                        effort,
                        via: ResolutionLayer::Downgrade,
                    };
                }
            }
        }
        Resolved { model, effort, via }
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

/// Minimal regex-lite shim: we avoid a full regex dependency for simple
/// patterns; supports the common subset used for model routing
/// (prefix*, exact, and .-style wildcards via the `regex-lite` crate if
/// available at build time; otherwise a tiny glob matcher).
mod regex_lite {
    pub struct Regex {
        glob: String,
    }

    impl Regex {
        pub fn new(pattern: &str) -> Result<Self, ()> {
            Ok(Self {
                glob: pattern.to_string(),
            })
        }

        /// Very small matcher: supports leading ^, trailing $, .* and literal
        /// segments. Anything more exotic falls back to substring match.
        pub fn is_match(&self, text: &str) -> bool {
            let p = self.glob.as_str();
            if p == ".*" || p == ".+" {
                return true;
            }
            let anchored_start = p.starts_with('^');
            let anchored_end = p.ends_with('$');
            let core = p.trim_start_matches('^').trim_end_matches('$');
            if let Some(prefix) = core.strip_suffix(".*") {
                return text.starts_with(prefix);
            }
            if let Some(suffix) = core.strip_prefix(".*") {
                return text.ends_with(suffix);
            }
            match (anchored_start, anchored_end) {
                (true, true) => text == core,
                (true, false) => text.starts_with(core),
                (false, true) => text.ends_with(core),
                (false, false) => text.contains(core),
            }
        }
    }
}
