use std::collections::HashMap;

/// Max suggestions kept per field name. Old entries are evicted LRU-style.
const MAX_PER_FIELD: usize = 20;

/// Field names never cached — auto-injected by [`super::types::ManagedFieldsPolicy`]
/// instead, so surfacing it as a "suggestion" would be misleading.
const MANAGED_FIELDS: &[&str] = &["account_id"];

/// Field names that pre-fill a form as a default (carry the last-used value
/// forward) rather than only being offered as a cyclable suggestion — the
/// "thing you're working with" identifiers you typically reuse call after
/// call in one session. Deliberately excludes `entity_id` (still cycled —
/// its own defaulting is a judgment call, see the field's own history) and
/// `invocation_id` (minted fresh per `run_agent` call; carrying it forward
/// would default to referencing a stale run, not a genuinely reusable id).
const DEFAULT_CARRY_FORWARD_FIELDS: &[&str] = &["station_id", "robot_id", "device_id", "agent_id"];

/// Discovery tools that can auto-populate a carry-forward field the first
/// time it's needed, tried in order — the first one present in the
/// currently-loaded tool list wins (an Account-scoped key sees
/// `account_devices`; a Station-scoped key only sees `station_devices`, etc.,
/// so which candidate exists depends on the caller's entity type).
pub const AUTO_FETCH_CANDIDATES: &[(&str, &[&str])] = &[
    ("device_id", &["account_devices", "station_devices", "robot_devices"]),
    ("station_id", &["account_stations"]),
    ("robot_id", &["account_robots"]),
    ("agent_id", &["list_agents"]),
];

/// Passive cache of `*_id` values seen in tool call results, keyed by the
/// JSON field name they appeared under (e.g. `device_id`, `agent_id`).
///
/// Populated by scanning every `McpToolResult` as it arrives — no tool gets
/// special-cased, so a form field named `device_id` gets suggestions the
/// first time *any* result carries a `device_id`, not just from
/// `account_devices`. Most-recently-seen value is first.
#[derive(Debug, Clone, Default)]
pub struct IdCache {
    by_field: HashMap<String, Vec<String>>,
}

impl IdCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Suggestions for a field name, most-recent first. Empty if none seen.
    pub fn suggestions(&self, field_name: &str) -> &[String] {
        self.by_field
            .get(field_name)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// The value to pre-fill a form field with, if `field_name` is one of
    /// [`DEFAULT_CARRY_FORWARD_FIELDS`] and something has been seen for it —
    /// i.e. the most-recently-used value, carried forward. `None` for every
    /// other field name, even if suggestions exist (they're still offered
    /// via [`Self::suggestions`], just not pre-filled).
    pub fn default_for(&self, field_name: &str) -> Option<&str> {
        if !DEFAULT_CARRY_FORWARD_FIELDS.contains(&field_name) {
            return None;
        }
        self.suggestions(field_name).first().map(String::as_str)
    }

    /// Record a value seen under `field_name`. Moves it to the front if
    /// already present (dedup), otherwise inserts and evicts the oldest
    /// entry once over `MAX_PER_FIELD`.
    fn record(&mut self, field_name: &str, value: &str) {
        if value.is_empty() || MANAGED_FIELDS.contains(&field_name) {
            return;
        }
        let entries = self.by_field.entry(field_name.to_string()).or_default();
        entries.retain(|v| v != value);
        entries.insert(0, value.to_string());
        entries.truncate(MAX_PER_FIELD);
    }

    /// Recursively scan a JSON value (object/array of any shape) for string
    /// fields whose key is an `*_id` identifier, recording each into the
    /// cache under its snake_case field name.
    ///
    /// Tool call *arguments* are snake_case (`device_id`, matching the form
    /// field name), but tool call *results* — confirmed against a live
    /// server — come back camelCase (`deviceId`). Both are normalized to
    /// snake_case here so a result's `deviceId` populates the same cache
    /// entry a `device_id` form field looks up.
    pub fn scan(&mut self, value: &serde_json::Value) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, v) in map {
                    let normalized = to_snake_case(key);
                    if normalized.ends_with("_id") {
                        if let serde_json::Value::String(s) = v {
                            self.record(&normalized, s);
                        }
                    }
                    self.scan(v);
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    self.scan(item);
                }
            }
            _ => {}
        }
    }
}

/// `deviceId` -> `device_id`. A no-op on keys that are already snake_case.
fn to_snake_case(key: &str) -> String {
    let mut out = String::with_capacity(key.len() + 4);
    for ch in key.chars() {
        if ch.is_ascii_uppercase() {
            out.push('_');
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_carry_forward_field_has_auto_fetch_candidates() {
        for field in DEFAULT_CARRY_FORWARD_FIELDS {
            let candidates = AUTO_FETCH_CANDIDATES
                .iter()
                .find(|(name, _)| name == field)
                .map(|(_, tools)| *tools);
            assert!(
                candidates.is_some_and(|c| !c.is_empty()),
                "{field} is carry-forward but has no auto-fetch candidate tool"
            );
        }
    }

    #[test]
    fn scan_records_top_level_id_fields() {
        let mut cache = IdCache::new();
        cache.scan(&json!({ "device_id": "D#1", "name": "gps-1" }));
        assert_eq!(cache.suggestions("device_id"), &["D#1"]);
    }

    #[test]
    fn scan_normalizes_camel_case_result_keys() {
        // Real server responses (confirmed against station_devices) use
        // camelCase, unlike snake_case request arguments — deviceId must
        // land under the same "device_id" cache entry a device_id form
        // field looks up, or the whole feature is silently inert.
        let mut cache = IdCache::new();
        cache.scan(&json!({
            "devices": [
                { "deviceId": "D#1", "parentId": "S#1" },
            ]
        }));
        assert_eq!(cache.suggestions("device_id"), &["D#1"]);
        assert_eq!(cache.suggestions("parent_id"), &["S#1"]);
        assert_eq!(cache.default_for("device_id"), Some("D#1"));
    }

    #[test]
    fn to_snake_case_is_a_no_op_on_already_snake_case_keys() {
        assert_eq!(to_snake_case("device_id"), "device_id");
        assert_eq!(to_snake_case("deviceId"), "device_id");
        assert_eq!(to_snake_case("invocationId"), "invocation_id");
    }

    #[test]
    fn default_for_carries_most_recent_value_forward() {
        let mut cache = IdCache::new();
        cache.scan(&json!({ "device_id": "D#1" }));
        cache.scan(&json!({ "device_id": "D#2" }));
        assert_eq!(cache.default_for("device_id"), Some("D#2"));

        for field in ["station_id", "robot_id", "agent_id"] {
            cache.scan(&json!({ field: "X#1" }));
            assert_eq!(cache.default_for(field), Some("X#1"));
        }
    }

    #[test]
    fn default_for_excludes_non_carry_forward_fields() {
        let mut cache = IdCache::new();
        cache.scan(&json!({ "entity_id": "d#1", "invocation_id": "inv-1" }));
        // Still cyclable via suggestions()...
        assert_eq!(cache.suggestions("entity_id"), &["d#1"]);
        assert_eq!(cache.suggestions("invocation_id"), &["inv-1"]);
        // ...but not pre-filled as a default.
        assert_eq!(cache.default_for("entity_id"), None);
        assert_eq!(cache.default_for("invocation_id"), None);
    }

    #[test]
    fn default_for_is_none_when_nothing_seen() {
        let cache = IdCache::new();
        assert_eq!(cache.default_for("device_id"), None);
    }

    #[test]
    fn scan_walks_nested_objects_and_arrays() {
        let mut cache = IdCache::new();
        cache.scan(&json!({
            "devices": [
                { "device_id": "D#1" },
                { "device_id": "D#2" },
            ],
            "entity": { "entity_id": "d#1" },
        }));
        assert_eq!(cache.suggestions("device_id"), &["D#2", "D#1"]);
        assert_eq!(cache.suggestions("entity_id"), &["d#1"]);
    }

    #[test]
    fn skips_managed_account_id() {
        let mut cache = IdCache::new();
        cache.scan(&json!({ "account_id": "A#1" }));
        assert!(cache.suggestions("account_id").is_empty());
    }

    #[test]
    fn most_recently_seen_is_first_and_dedups() {
        let mut cache = IdCache::new();
        cache.scan(&json!({ "device_id": "D#1" }));
        cache.scan(&json!({ "device_id": "D#2" }));
        cache.scan(&json!({ "device_id": "D#1" }));
        assert_eq!(cache.suggestions("device_id"), &["D#1", "D#2"]);
    }

    #[test]
    fn caps_entries_per_field() {
        let mut cache = IdCache::new();
        for i in 0..(MAX_PER_FIELD + 5) {
            cache.scan(&json!({ "device_id": format!("D#{i}") }));
        }
        assert_eq!(cache.suggestions("device_id").len(), MAX_PER_FIELD);
        // Most recent survives.
        assert_eq!(
            cache.suggestions("device_id")[0],
            format!("D#{}", MAX_PER_FIELD + 4)
        );
    }

    #[test]
    fn ignores_empty_string_values() {
        let mut cache = IdCache::new();
        cache.scan(&json!({ "device_id": "" }));
        assert!(cache.suggestions("device_id").is_empty());
    }
}
