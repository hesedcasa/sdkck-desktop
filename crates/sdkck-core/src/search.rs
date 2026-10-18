//! Native fuzzy command search (nucleo, the matcher behind helix/television):
//! the desktop counterpart of `sdkck search`'s fuzzy fallback, without a Node
//! process per keystroke.

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use crate::catalog::CommandMeta;

pub struct Searcher {
    matcher: Matcher,
}

impl Default for Searcher {
    fn default() -> Self {
        Searcher { matcher: Matcher::new(Config::DEFAULT) }
    }
}

impl Searcher {
    /// Indexes of `commands` matching `query`, best first. An empty query
    /// returns every command in catalog order.
    pub fn search(&mut self, commands: &[CommandMeta], query: &str, limit: usize) -> Vec<usize> {
        let query = query.trim();
        if query.is_empty() {
            return (0..commands.len().min(limit)).collect();
        }
        // Ids match fuzzily (`jis` → `jira issue search`); prose only by
        // substring, or every long description matches every short query.
        let pattern = Pattern::new(query, CaseMatching::Ignore, Normalization::Smart, AtomKind::Fuzzy);
        let prose = Pattern::new(query, CaseMatching::Ignore, Normalization::Smart, AtomKind::Substring);
        let lowered = query.to_lowercase();
        let mut buf = Vec::new();
        let mut scored: Vec<(u32, usize)> = commands
            .iter()
            .enumerate()
            .filter_map(|(i, cmd)| {
                let mut score = |p: &Pattern, text: &str, weight: u32| {
                    p.score(Utf32Str::new(text, &mut buf), &mut self.matcher).map(|s| s * weight)
                };
                let id = score(&pattern, &cmd.display_id, 4);
                let summary = cmd.summary.as_deref().and_then(|s| score(&prose, s, 2));
                let description = cmd.description.as_deref().and_then(|s| score(&prose, s, 1));
                let mut best = [id, summary, description].into_iter().flatten().max()?;
                // Typing a command's literal prefix should put it on top.
                if cmd.display_id.starts_with(&lowered) {
                    best += 10_000;
                }
                Some((best, i))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        scored.into_iter().take(limit).map(|(_, i)| i).collect()
    }
}
