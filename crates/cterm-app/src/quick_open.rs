//! Quick Open Template matching/filtering logic
//!
//! Provides fuzzy matching for quickly searching and opening tab templates.

use crate::config::StickyTabConfig;

/// Result of matching a template against a query
#[derive(Debug, Clone)]
pub struct TemplateMatch {
    /// The matching template
    pub template: StickyTabConfig,
    /// Match score (higher = better match)
    pub score: i32,
    /// Character positions that matched (for highlighting)
    pub match_positions: Vec<usize>,
}

/// Matcher for filtering templates by search query
pub struct QuickOpenMatcher {
    templates: Vec<StickyTabConfig>,
}

impl QuickOpenMatcher {
    /// Create a new matcher with the given templates
    pub fn new(templates: Vec<StickyTabConfig>) -> Self {
        Self { templates }
    }

    /// Filter templates by query string
    ///
    /// Returns matches sorted by score (best match first).
    /// Scoring:
    /// - Exact match: 1000
    /// - Prefix match: 500 + length bonus
    /// - Substring match: 200 + position bonus
    /// - Fuzzy match: 100 + consecutive bonus
    pub fn filter(&self, query: &str) -> Vec<TemplateMatch> {
        if query.is_empty() {
            // Return all templates with equal score when query is empty
            return self
                .templates
                .iter()
                .map(|t| TemplateMatch {
                    template: t.clone(),
                    score: 0,
                    match_positions: Vec::new(),
                })
                .collect();
        }

        let query_lower = query.to_lowercase();
        let mut matches: Vec<TemplateMatch> = self
            .templates
            .iter()
            .filter_map(|template| self.match_template(template, &query_lower))
            .collect();

        // Sort by score (descending), then by name (ascending)
        matches.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| a.template.name.cmp(&b.template.name))
        });

        matches
    }

    /// Try to match a single template against the query
    fn match_template(&self, template: &StickyTabConfig, query: &str) -> Option<TemplateMatch> {
        let name_lower = template.name.to_lowercase();

        // Exact match
        if name_lower == query {
            return Some(TemplateMatch {
                template: template.clone(),
                score: 1000,
                match_positions: (0..template.name.len()).collect(),
            });
        }

        // Prefix match
        if name_lower.starts_with(query) {
            return Some(TemplateMatch {
                template: template.clone(),
                score: 500 + query.len() as i32 * 10,
                match_positions: (0..query.len()).collect(),
            });
        }

        // Substring match
        if let Some(pos) = name_lower.find(query) {
            return Some(TemplateMatch {
                template: template.clone(),
                score: 200 + (100 - pos as i32).max(0),
                match_positions: (pos..pos + query.len()).collect(),
            });
        }

        // Fuzzy match - each query char must appear in order
        let mut match_positions = Vec::new();
        let mut name_chars = name_lower.char_indices().peekable();
        let mut last_match_pos: Option<usize> = None;
        let mut consecutive_bonus = 0i32;

        for query_char in query.chars() {
            let mut found = false;
            for (pos, name_char) in name_chars.by_ref() {
                if name_char == query_char {
                    // Bonus for consecutive matches
                    if let Some(last) = last_match_pos {
                        if pos == last + 1 {
                            consecutive_bonus += 20;
                        }
                    }
                    match_positions.push(pos);
                    last_match_pos = Some(pos);
                    found = true;
                    break;
                }
            }
            if !found {
                return None;
            }
        }

        Some(TemplateMatch {
            template: template.clone(),
            score: 100 + consecutive_bonus + match_positions.len() as i32 * 5,
            match_positions,
        })
    }

    /// Get the number of templates
    pub fn len(&self) -> usize {
        self.templates.len()
    }

    /// Check if there are no templates
    pub fn is_empty(&self) -> bool {
        self.templates.is_empty()
    }

    /// Get all templates
    pub fn templates(&self) -> &[StickyTabConfig] {
        &self.templates
    }
}

/// Get the type indicator emoji for a template
pub fn template_type_indicator(template: &StickyTabConfig) -> &'static str {
    if template.remote.is_some() {
        "\u{1F310}" // Globe emoji for remote daemon
    } else if template.docker.is_some() {
        "\u{1F433}" // Whale emoji for Docker
    } else if template.ssh.is_some() {
        "\u{1F517}" // Link emoji for SSH
    } else {
        ""
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_templates() -> Vec<StickyTabConfig> {
        vec![
            StickyTabConfig {
                name: "Claude".into(),
                ..Default::default()
            },
            StickyTabConfig {
                name: "Ubuntu Container".into(),
                ..Default::default()
            },
            StickyTabConfig {
                name: "SSH to prod-server".into(),
                ..Default::default()
            },
            StickyTabConfig {
                name: "Python Dev".into(),
                ..Default::default()
            },
            StickyTabConfig {
                name: "Default Shell".into(),
                ..Default::default()
            },
        ]
    }

    #[test]
    fn test_empty_query_returns_all() {
        let matcher = QuickOpenMatcher::new(make_templates());
        let results = matcher.filter("");
        assert_eq!(results.len(), 5);
    }

    #[test]
    fn test_exact_match() {
        let matcher = QuickOpenMatcher::new(make_templates());
        let results = matcher.filter("Claude");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].template.name, "Claude");
        assert_eq!(results[0].score, 1000);
    }

    #[test]
    fn test_prefix_match() {
        let matcher = QuickOpenMatcher::new(make_templates());
        let results = matcher.filter("Ub");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].template.name, "Ubuntu Container");
        assert!(results[0].score >= 500);
    }

    #[test]
    fn test_substring_match() {
        let matcher = QuickOpenMatcher::new(make_templates());
        let results = matcher.filter("Container");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].template.name, "Ubuntu Container");
    }

    #[test]
    fn test_fuzzy_match() {
        let matcher = QuickOpenMatcher::new(make_templates());
        let results = matcher.filter("pd");
        // Should match "Python Dev" with p...d fuzzy match
        assert!(results.iter().any(|r| r.template.name == "Python Dev"));
    }

    #[test]
    fn test_case_insensitive() {
        let matcher = QuickOpenMatcher::new(make_templates());
        let results = matcher.filter("claude");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].template.name, "Claude");
    }

    #[test]
    fn test_no_match() {
        let matcher = QuickOpenMatcher::new(make_templates());
        let results = matcher.filter("xyz");
        assert!(results.is_empty());
    }

    #[test]
    fn test_sorting_by_score() {
        let matcher = QuickOpenMatcher::new(make_templates());
        let results = matcher.filter("De");
        // Should have "Default Shell" first (prefix match) then "Python Dev" (contains "Dev")
        assert!(!results.is_empty());
        if results.len() > 1 {
            assert!(results[0].score >= results[1].score);
        }
    }
}
