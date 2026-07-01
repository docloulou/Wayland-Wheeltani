//! Case-insensitive allow/deny matching of the foreground application.

use super::config::{ForegroundConfig, ForegroundMode};
use super::filter::{AutoscrollDecision, ForegroundApp};

/// Normalises an identifier for comparison: trims surrounding whitespace, drops
/// a trailing `.desktop`, and lowercases (ASCII).
#[must_use]
pub fn normalize(s: &str) -> String {
    s.trim().trim_end_matches(".desktop").to_ascii_lowercase()
}

/// Returns true if any populated identity field of `app` matches an entry in
/// `list`. `title` is only considered when `match_title` is set.
fn app_matches(app: &ForegroundApp, list: &[String], match_title: bool) -> bool {
    if list.is_empty() {
        return false;
    }
    let needles: Vec<String> = list.iter().map(|s| normalize(s)).collect();

    let mut fields: Vec<&str> = Vec::with_capacity(4);
    if let Some(v) = app.app_id.as_deref() {
        fields.push(v);
    }
    if let Some(v) = app.class.as_deref() {
        fields.push(v);
    }
    if let Some(v) = app.resource_class.as_deref() {
        fields.push(v);
    }
    if match_title {
        if let Some(v) = app.title.as_deref() {
            fields.push(v);
        }
    }

    fields
        .iter()
        .map(|f| normalize(f))
        .any(|f| needles.contains(&f))
}

/// Applies the configured policy to `app`.
#[must_use]
pub fn decide(app: &ForegroundApp, cfg: &ForegroundConfig) -> AutoscrollDecision {
    match cfg.mode {
        ForegroundMode::Denylist => {
            if app_matches(app, &cfg.deny_apps, cfg.match_title) {
                AutoscrollDecision::Disabled
            } else {
                AutoscrollDecision::Enabled
            }
        }
        ForegroundMode::Allowlist => {
            if app_matches(app, &cfg.allow_apps, cfg.match_title) {
                AutoscrollDecision::Enabled
            } else {
                AutoscrollDecision::Disabled
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foreground::config::{ForegroundConfig, ForegroundMode};
    use crate::foreground::filter::{AutoscrollDecision, ForegroundApp, ForegroundSourceKind};

    fn app_with_class(class: &str) -> ForegroundApp {
        ForegroundApp {
            app_id: None,
            class: Some(class.to_owned()),
            resource_class: None,
            title: None,
            pid: None,
            source: ForegroundSourceKind::Unknown,
        }
    }

    fn denylist(entries: &[&str]) -> ForegroundConfig {
        ForegroundConfig {
            enabled: true,
            mode: ForegroundMode::Denylist,
            deny_apps: entries.iter().map(|s| (*s).to_owned()).collect(),
            ..ForegroundConfig::default()
        }
    }

    fn allowlist(entries: &[&str]) -> ForegroundConfig {
        ForegroundConfig {
            enabled: true,
            mode: ForegroundMode::Allowlist,
            allow_apps: entries.iter().map(|s| (*s).to_owned()).collect(),
            ..ForegroundConfig::default()
        }
    }

    #[test]
    fn normalize_lowercases_trims_and_drops_desktop_suffix() {
        assert_eq!(normalize("  Firefox.desktop "), "firefox");
        assert_eq!(normalize("Org.Mozilla.Firefox"), "org.mozilla.firefox");
    }

    #[test]
    fn denylist_match_is_case_insensitive() {
        let cfg = denylist(&["FireFox"]);
        assert_eq!(
            decide(&app_with_class("firefox"), &cfg),
            AutoscrollDecision::Disabled
        );
    }

    #[test]
    fn denylist_non_match_stays_enabled() {
        let cfg = denylist(&["firefox"]);
        assert_eq!(
            decide(&app_with_class("code"), &cfg),
            AutoscrollDecision::Enabled
        );
    }

    #[test]
    fn allowlist_only_enables_listed_apps() {
        let cfg = allowlist(&["code"]);
        assert_eq!(
            decide(&app_with_class("code"), &cfg),
            AutoscrollDecision::Enabled
        );
        assert_eq!(
            decide(&app_with_class("firefox"), &cfg),
            AutoscrollDecision::Disabled
        );
    }

    #[test]
    fn desktop_suffix_ignored_on_both_sides() {
        let cfg = denylist(&["firefox.desktop"]);
        assert_eq!(
            decide(&app_with_class("firefox"), &cfg),
            AutoscrollDecision::Disabled
        );
    }

    #[test]
    fn title_ignored_unless_match_title() {
        let app = ForegroundApp {
            app_id: None,
            class: Some("code".to_owned()),
            resource_class: None,
            title: Some("firefox".to_owned()),
            pid: None,
            source: ForegroundSourceKind::Unknown,
        };

        let cfg = denylist(&["firefox"]);
        assert_eq!(decide(&app, &cfg), AutoscrollDecision::Enabled);

        let cfg_title = ForegroundConfig {
            match_title: true,
            ..denylist(&["firefox"])
        };
        assert_eq!(decide(&app, &cfg_title), AutoscrollDecision::Disabled);
    }

    #[test]
    fn matches_app_id_and_resource_class_fields() {
        let app = ForegroundApp {
            app_id: Some("org.mozilla.firefox".to_owned()),
            class: None,
            resource_class: Some("Navigator".to_owned()),
            title: None,
            pid: None,
            source: ForegroundSourceKind::Unknown,
        };
        assert_eq!(
            decide(&app, &denylist(&["org.mozilla.firefox"])),
            AutoscrollDecision::Disabled
        );
        assert_eq!(
            decide(&app, &denylist(&["navigator"])),
            AutoscrollDecision::Disabled
        );
    }
}
