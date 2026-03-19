use crate::config::Config;
use crate::error::SearchError;
use crate::markdown::{extract_field, split_frontmatter, Frontmatter, IntentKind};
use serde::Serialize;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

// ──────────────────────────────────────────────────────────────
// Built-in search: walk dir + case-insensitive text match.
// Zero dependencies beyond walkdir. Fast enough for <1000 files.
//
// Config can swap to QMD engine for semantic search:
//   [search]
//   engine = "qmd"
//   qmd_collection = "meetings"
// ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    pub path: PathBuf,
    pub title: String,
    pub date: String,
    pub content_type: String,
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct IntentResult {
    pub path: PathBuf,
    pub title: String,
    pub date: String,
    pub content_type: String,
    pub kind: IntentKind,
    pub what: String,
    pub who: Option<String>,
    pub status: String,
    pub by_date: Option<String>,
}

pub struct SearchFilters {
    pub content_type: Option<String>,
    pub since: Option<String>,
    pub attendee: Option<String>,
}

/// Search all markdown files in the meetings directory.
pub fn search(
    query: &str,
    config: &Config,
    filters: &SearchFilters,
) -> Result<Vec<SearchResult>, SearchError> {
    let dir = &config.output_dir;
    if !dir.exists() {
        return Err(SearchError::DirNotFound(dir.display().to_string()));
    }

    let query_lower = query.to_lowercase();
    let mut results = Vec::new();

    for entry in WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "md"))
    {
        let path = entry.path();
        match process_file(path, &query_lower, filters) {
            Ok(Some(result)) => results.push(result),
            Ok(None) => {} // No match
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "skipping file in search");
            }
        }
    }

    // Sort by date descending (newest first)
    results.sort_by(|a, b| b.date.cmp(&a.date));
    Ok(results)
}

/// Search structured intents across all markdown files in the meetings directory.
pub fn search_intents(
    query: &str,
    config: &Config,
    filters: &SearchFilters,
) -> Result<Vec<IntentResult>, SearchError> {
    let dir = &config.output_dir;
    if !dir.exists() {
        return Err(SearchError::DirNotFound(dir.display().to_string()));
    }

    let query_lower = query.to_lowercase();
    let mut results = Vec::new();

    for entry in WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "md"))
    {
        let path = entry.path();
        match process_intent_file(path, &query_lower, filters) {
            Ok(mut file_results) => results.append(&mut file_results),
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "skipping file in intent search");
            }
        }
    }

    results.sort_by(|a, b| b.date.cmp(&a.date));
    Ok(results)
}

fn process_file(
    path: &Path,
    query: &str,
    filters: &SearchFilters,
) -> Result<Option<SearchResult>, SearchError> {
    let content = std::fs::read_to_string(path)?;

    // Parse frontmatter
    let (frontmatter_str, body) = split_frontmatter(&content);
    let title = extract_field(frontmatter_str, "title").unwrap_or_default();
    let date = extract_field(frontmatter_str, "date").unwrap_or_default();
    let content_type = extract_field(frontmatter_str, "type").unwrap_or_else(|| "meeting".into());

    // Apply filters
    if let Some(ref type_filter) = filters.content_type {
        if content_type != *type_filter {
            return Ok(None);
        }
    }
    if let Some(ref since) = filters.since {
        if date < *since {
            return Ok(None);
        }
    }
    if let Some(ref attendee) = filters.attendee {
        let attendees = extract_field(frontmatter_str, "attendees").unwrap_or_default();
        if !attendees.to_lowercase().contains(&attendee.to_lowercase()) {
            return Ok(None);
        }
    }

    // Text search (case-insensitive)
    let body_lower = body.to_lowercase();
    let title_lower = title.to_lowercase();

    if body_lower.contains(query) || title_lower.contains(query) {
        let snippet = extract_snippet(body, query);
        Ok(Some(SearchResult {
            path: path.to_path_buf(),
            title,
            date,
            content_type,
            snippet,
        }))
    } else {
        Ok(None)
    }
}

fn process_intent_file(
    path: &Path,
    query: &str,
    filters: &SearchFilters,
) -> Result<Vec<IntentResult>, SearchError> {
    let content = std::fs::read_to_string(path)?;
    let (frontmatter_str, _) = split_frontmatter(&content);
    if frontmatter_str.is_empty() {
        return Ok(vec![]);
    }

    let frontmatter: Frontmatter = serde_yaml::from_str(frontmatter_str)
        .map_err(|e| SearchError::Io(std::io::Error::other(e.to_string())))?;

    let date = frontmatter.date.to_rfc3339();
    let content_type = match frontmatter.r#type {
        crate::markdown::ContentType::Meeting => "meeting".to_string(),
        crate::markdown::ContentType::Memo => "memo".to_string(),
    };

    if let Some(ref type_filter) = filters.content_type {
        if content_type != *type_filter {
            return Ok(vec![]);
        }
    }
    if let Some(ref since) = filters.since {
        if date < *since {
            return Ok(vec![]);
        }
    }
    if let Some(ref attendee) = filters.attendee {
        let attendee_lower = attendee.to_lowercase();
        let attendee_match = frontmatter
            .attendees
            .iter()
            .any(|name| name.to_lowercase().contains(&attendee_lower));
        if !attendee_match {
            return Ok(vec![]);
        }
    }

    let mut results = Vec::new();
    for intent in frontmatter.intents {
        let haystack = format!(
            "{} {} {} {} {}",
            frontmatter.title,
            intent.what,
            intent.who.clone().unwrap_or_default(),
            intent.status,
            intent.by_date.clone().unwrap_or_default()
        )
        .to_lowercase();

        if !query.is_empty() && !haystack.contains(query) {
            continue;
        }

        results.push(IntentResult {
            path: path.to_path_buf(),
            title: frontmatter.title.clone(),
            date: date.clone(),
            content_type: content_type.clone(),
            kind: intent.kind,
            what: intent.what,
            who: intent.who,
            status: intent.status,
            by_date: intent.by_date,
        });
    }

    Ok(results)
}

// split_frontmatter and extract_field are in markdown.rs (shared)

/// Find meetings with open action items, optionally filtered by assignee.
/// Parses YAML frontmatter for the structured action_items field.
pub fn find_open_actions(
    config: &Config,
    assignee: Option<&str>,
) -> Result<Vec<ActionResult>, SearchError> {
    let dir = &config.output_dir;
    if !dir.exists() {
        return Ok(vec![]);
    }

    let mut results = Vec::new();

    for entry in WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "md"))
    {
        let path = entry.path();
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let (fm_str, _) = split_frontmatter(&content);
        let title = extract_field(fm_str, "title").unwrap_or_default();
        let date = extract_field(fm_str, "date").unwrap_or_default();

        // Parse action_items from frontmatter (YAML list)
        // Look for lines like "  - assignee: mat" within the action_items block
        if !content.contains("action_items:") {
            continue;
        }

        // Simple parse: find action_items section in frontmatter YAML
        let full_fm = format!("---\n{}\n---", fm_str);
        let parsed: Result<serde_yaml::Value, _> = serde_yaml::from_str(&full_fm);
        if let Ok(yaml) = parsed {
            if let Some(items) = yaml.get("action_items").and_then(|v| v.as_sequence()) {
                for item in items {
                    let item_assignee = item
                        .get("assignee")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unassigned");
                    let item_status = item
                        .get("status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("open");
                    let item_task = item.get("task").and_then(|v| v.as_str()).unwrap_or("");
                    let item_due = item
                        .get("due")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());

                    if item_status != "open" {
                        continue;
                    }
                    if let Some(filter) = assignee {
                        if !item_assignee.eq_ignore_ascii_case(filter) {
                            continue;
                        }
                    }

                    results.push(ActionResult {
                        meeting_path: path.to_path_buf(),
                        meeting_title: title.clone(),
                        meeting_date: date.clone(),
                        assignee: item_assignee.to_string(),
                        task: item_task.to_string(),
                        due: item_due,
                    });
                }
            }
        }
    }

    results.sort_by(|a, b| b.meeting_date.cmp(&a.meeting_date));
    Ok(results)
}

/// A structured action item result from cross-meeting search.
#[derive(Debug, Clone, Serialize)]
pub struct ActionResult {
    pub meeting_path: PathBuf,
    pub meeting_title: String,
    pub meeting_date: String,
    pub assignee: String,
    pub task: String,
    pub due: Option<String>,
}

/// Extract a snippet around the first match of the query.
fn extract_snippet(body: &str, query: &str) -> String {
    // Find the query in the body case-insensitively.
    // We search the original body to avoid byte-offset mismatch from to_lowercase().
    let pos = body
        .char_indices()
        .position(|(i, _)| body[i..].to_lowercase().starts_with(query))
        .and_then(|char_idx| body.char_indices().nth(char_idx).map(|(i, _)| i));

    if let Some(pos) = pos {
        let start = body[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let end = body[pos..]
            .find('\n')
            .map(|i| pos + i)
            .unwrap_or(body.len());

        let line = body[start..end].trim();
        if line.chars().count() > 200 {
            let truncated: String = line.chars().take(200).collect();
            format!("{}...", truncated)
        } else {
            line.to_string()
        }
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_file(dir: &Path, name: &str, content: &str) {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }

    #[test]
    fn search_finds_matching_content() {
        let dir = TempDir::new().unwrap();
        create_test_file(
            dir.path(),
            "2026-03-17-test.md",
            "---\ntitle: Test Meeting\ndate: 2026-03-17\ntype: meeting\n---\n\n## Transcript\n\nWe discussed pricing strategy in detail.",
        );

        let config = Config {
            output_dir: dir.path().to_path_buf(),
            ..Config::default()
        };
        let filters = SearchFilters {
            content_type: None,
            since: None,
            attendee: None,
        };

        let results = search("pricing", &config, &filters).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].snippet.contains("pricing"));
    }

    #[test]
    fn search_returns_empty_for_no_match() {
        let dir = TempDir::new().unwrap();
        create_test_file(
            dir.path(),
            "test.md",
            "---\ntitle: Test\ndate: 2026-03-17\n---\n\nHello world.",
        );

        let config = Config {
            output_dir: dir.path().to_path_buf(),
            ..Config::default()
        };
        let filters = SearchFilters {
            content_type: None,
            since: None,
            attendee: None,
        };

        let results = search("nonexistent", &config, &filters).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn search_is_case_insensitive() {
        let dir = TempDir::new().unwrap();
        create_test_file(
            dir.path(),
            "test.md",
            "---\ntitle: Test\ndate: 2026-03-17\n---\n\nPRICING discussion",
        );

        let config = Config {
            output_dir: dir.path().to_path_buf(),
            ..Config::default()
        };
        let filters = SearchFilters {
            content_type: None,
            since: None,
            attendee: None,
        };

        let results = search("pricing", &config, &filters).unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn search_empty_directory() {
        let dir = TempDir::new().unwrap();
        let config = Config {
            output_dir: dir.path().to_path_buf(),
            ..Config::default()
        };
        let filters = SearchFilters {
            content_type: None,
            since: None,
            attendee: None,
        };

        let results = search("anything", &config, &filters).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn split_frontmatter_works() {
        let content = "---\ntitle: Test\ndate: 2026-03-17\n---\n\nBody text here.";
        let (fm, body) = split_frontmatter(content);
        assert!(fm.contains("title: Test"));
        assert!(body.contains("Body text here"));
    }

    #[test]
    fn extract_field_finds_value() {
        let fm = "title: My Meeting\ndate: 2026-03-17\ntype: meeting";
        assert_eq!(extract_field(fm, "title"), Some("My Meeting".into()));
        assert_eq!(extract_field(fm, "type"), Some("meeting".into()));
        assert_eq!(extract_field(fm, "nonexistent"), None);
    }

    #[test]
    fn search_intents_returns_matching_structured_records() {
        let dir = TempDir::new().unwrap();
        create_test_file(
            dir.path(),
            "2026-03-17-test.md",
            "---\ntitle: Pricing Review\ntype: meeting\ndate: 2026-03-17T12:00:00-07:00\nduration: 42m\nstatus: complete\ntags: []\nattendees: []\npeople: []\naction_items: []\ndecisions: []\nintents:\n  - kind: action-item\n    what: Send pricing doc\n    who: mat\n    status: open\n    by_date: Friday\n  - kind: commitment\n    what: Share revised pricing model\n    who: sarah\n    status: open\n    by_date: Tuesday\n---\n\n## Transcript\n\nWe discussed pricing.\n",
        );

        let config = Config {
            output_dir: dir.path().to_path_buf(),
            ..Config::default()
        };
        let filters = SearchFilters {
            content_type: None,
            since: None,
            attendee: None,
        };

        let results = process_intent_file(
            &dir.path().join("2026-03-17-test.md"),
            "pricing",
            &filters,
        )
        .unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Pricing Review");
        assert!(results
            .iter()
            .any(|item| item.kind == IntentKind::ActionItem));
        assert!(results
            .iter()
            .any(|item| item.kind == IntentKind::Commitment));
    }
}
