//! Scenario tags that gate where a scenario runs (PLAN.md §3, docs/steps.md).

use cucumber::gherkin::{Feature, Rule, Scenario};

/// Scenario tags naming an operating system.
const PLATFORM_TAGS: [&str; 3] = ["linux", "macos", "windows"];

/// Scenario tags naming a capability the job must opt into.
const CAPABILITY_TAGS: [&str; 7] = [
    "mount",
    "winfsp",
    "providers",
    "privileged",
    "slow",
    "claude",
    "lspmux",
];

/// Environment variable listing the capabilities a job provides, comma-separated.
pub const CAPABILITIES_ENV: &str = "CODETAGS_BDD_CAPABILITIES";

/// Whether a scenario carrying `tags` runs on `os` with `capabilities`.
///
/// Platform tags restrict a scenario to the named OSes; with none it runs on
/// every OS. Each capability tag must appear in `capabilities`.
pub fn scenario_enabled(tags: &[String], os: &str, capabilities: &[String]) -> bool {
    let platforms: Vec<&str> = tags
        .iter()
        .map(String::as_str)
        .filter(|tag| PLATFORM_TAGS.contains(tag))
        .collect();
    let platform_ok = platforms.is_empty() || platforms.contains(&os);
    let capabilities_ok = tags
        .iter()
        .filter(|tag| CAPABILITY_TAGS.contains(&tag.as_str()))
        .all(|tag| capabilities.contains(tag));
    platform_ok && capabilities_ok
}

fn capabilities_from_env() -> Vec<String> {
    std::env::var(CAPABILITIES_ENV)
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|cap| !cap.is_empty())
        .map(str::to_string)
        .collect()
}

/// Cucumber's scenario filter: applies [`scenario_enabled`] to the tags of
/// the feature, the rule, and the scenario, and reports what it leaves out.
pub(crate) fn filter(feature: &Feature, rule: Option<&Rule>, scenario: &Scenario) -> bool {
    let tags: Vec<String> = feature
        .tags
        .iter()
        .chain(rule.map(|rule| rule.tags.iter()).into_iter().flatten())
        .chain(scenario.tags.iter())
        .cloned()
        .collect();
    let enabled = scenario_enabled(&tags, std::env::consts::OS, &capabilities_from_env());
    if !enabled {
        eprintln!(
            "bdd: not run here: {}: {} (tags: {})",
            feature.name,
            scenario.name,
            tags.join(" ")
        );
    }
    enabled
}

#[cfg(test)]
mod tests {
    use super::scenario_enabled;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    #[test]
    fn untagged_scenarios_run_everywhere() {
        assert!(scenario_enabled(&[], "windows", &[]));
    }

    #[test]
    fn platform_tags_restrict_to_the_named_oses() {
        let tags = strings(&["linux", "macos"]);
        assert!(scenario_enabled(&tags, "macos", &[]));
        assert!(!scenario_enabled(&tags, "windows", &[]));
    }

    #[test]
    fn capability_tags_need_every_capability() {
        let tags = strings(&["mount", "slow"]);
        assert!(!scenario_enabled(&tags, "linux", &strings(&["mount"])));
        assert!(scenario_enabled(
            &tags,
            "linux",
            &strings(&["slow", "mount"])
        ));
    }

    #[test]
    fn unrelated_tags_do_not_gate() {
        assert!(scenario_enabled(&strings(&["wip-note"]), "linux", &[]));
    }
}
