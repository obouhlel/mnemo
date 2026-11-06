//! Three-way merge used to resolve edit conflicts, GitHub style.
//!
//! * base   = last version of the note this instance knew was saved in the database
//! * mine   = what is currently in the local editor
//! * theirs = the newer version found in the database (another device / window / SQL)
//!
//! Non-overlapping edits are merged automatically; overlapping ones become
//! [`Block::Conflict`] that the user resolves hunk by hunk.

use similar::{ChangeTag, MergeResolution, TextDiff, TextMerge};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Unchanged,
    Mine,
    Theirs,
    Both,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    Unresolved,
    Mine,
    Theirs,
    MineThenTheirs,
    TheirsThenMine,
    /// Hand-edited text (initialised from a proposal)
    Custom,
}

#[derive(Debug, Clone)]
pub enum Block {
    Text { text: String, origin: Origin },
    Conflict { base: String, mine: String, theirs: String, choice: Choice, custom: String },
}

fn join_lines<'a>(lines: impl Iterator<Item = &'a str>) -> String {
    lines.collect()
}

fn with_trailing_newline(s: &str) -> String {
    if s.is_empty() || s.ends_with('\n') { s.to_string() } else { format!("{s}\n") }
}

pub fn concat(first: &str, second: &str) -> String {
    format!("{}{}", with_trailing_newline(first), second)
}

/// Builds the list of merge blocks.
pub fn build(base: &str, mine: &str, theirs: &str) -> Vec<Block> {
    let merge = TextMerge::from_lines(base, mine, theirs);
    let mut blocks: Vec<Block> = vec![];
    for region in merge.regions() {
        let b = join_lines(region.base_range().filter_map(|i| merge.base_line(i)));
        let m = join_lines(region.ours_range().filter_map(|i| merge.ours_line(i)));
        let t = join_lines(region.theirs_range().filter_map(|i| merge.theirs_line(i)));
        let block = match region.resolution() {
            MergeResolution::Unchanged => Block::Text { text: b, origin: Origin::Unchanged },
            MergeResolution::Ours => Block::Text { text: m, origin: Origin::Mine },
            MergeResolution::Theirs => Block::Text { text: t, origin: Origin::Theirs },
            MergeResolution::Both => Block::Text { text: m, origin: Origin::Both },
            _ => Block::Conflict { base: b, mine: m, theirs: t, choice: Choice::Unresolved, custom: String::new() },
        };
        // coalesce consecutive text blocks of the same origin
        if let (Some(Block::Text { text: prev, origin: po }), Block::Text { text, origin }) =
            (blocks.last_mut(), &block)
            && po == origin
        {
            prev.push_str(text);
            continue;
        }
        blocks.push(block);
    }
    blocks
}

pub fn resolved_text(mine: &str, theirs: &str, choice: &Choice, custom: &str) -> Option<String> {
    Some(match choice {
        Choice::Unresolved => return None,
        Choice::Mine => mine.to_string(),
        Choice::Theirs => theirs.to_string(),
        Choice::MineThenTheirs => concat(mine, theirs),
        Choice::TheirsThenMine => concat(theirs, mine),
        Choice::Custom => custom.to_string(),
    })
}

/// The merged document, or `None` while some conflict is unresolved.
pub fn assemble(blocks: &[Block]) -> Option<String> {
    let mut out = String::new();
    for b in blocks {
        let piece = match b {
            Block::Text { text, .. } => text.clone(),
            Block::Conflict { mine, theirs, choice, custom, .. } => resolved_text(mine, theirs, choice, custom)?,
        };
        // keep line structure intact when a hunk lost its final newline
        if !out.is_empty() && !out.ends_with('\n') && !piece.is_empty() {
            out.push('\n');
        }
        out.push_str(&piece);
    }
    Some(out)
}

pub fn conflict_count(blocks: &[Block]) -> (usize, usize) {
    let total = blocks.iter().filter(|b| matches!(b, Block::Conflict { .. })).count();
    let open = blocks.iter().filter(|b| matches!(b, Block::Conflict { choice: Choice::Unresolved, .. })).count();
    (total, open)
}

/// One line of a unified diff (GitHub style). `None` line entries separate hunks.
#[derive(Debug, Clone)]
pub struct DiffLine {
    pub tag: ChangeTag,
    pub old_no: Option<usize>,
    pub new_no: Option<usize>,
    pub text: String,
}

/// Unified diff from `old` to `new`, grouped in hunks with `context` lines of context.
pub fn unified(old: &str, new: &str, context: usize) -> Vec<Vec<DiffLine>> {
    let diff = TextDiff::from_lines(old, new);
    diff.grouped_ops(context)
        .iter()
        .map(|group| {
            group
                .iter()
                .flat_map(|op| diff.iter_changes(op))
                .map(|c| DiffLine {
                    tag: c.tag(),
                    old_no: c.old_index().map(|i| i + 1),
                    new_no: c.new_index().map(|i| i + 1),
                    text: c.value().trim_end_matches('\n').to_string(),
                })
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_merges_disjoint_edits() {
        let base = "a\nb\nc\nd\n";
        let mine = "A\nb\nc\nd\n";
        let theirs = "a\nb\nc\nD\n";
        let blocks = build(base, mine, theirs);
        assert_eq!(conflict_count(&blocks), (0, 0));
        assert_eq!(assemble(&blocks).unwrap(), "A\nb\nc\nD\n");
    }

    #[test]
    fn conflicts_and_choices() {
        let base = "titre\nligne\nfin\n";
        let mine = "titre\nligne locale\nfin\n";
        let theirs = "titre\nligne distante\nfin\n";
        let mut blocks = build(base, mine, theirs);
        assert_eq!(conflict_count(&blocks), (1, 1));
        assert!(assemble(&blocks).is_none());
        for b in &mut blocks {
            if let Block::Conflict { choice, .. } = b {
                *choice = Choice::MineThenTheirs;
            }
        }
        assert_eq!(assemble(&blocks).unwrap(), "titre\nligne locale\nligne distante\nfin\n");
        for b in &mut blocks {
            if let Block::Conflict { choice, .. } = b {
                *choice = Choice::Theirs;
            }
        }
        assert_eq!(assemble(&blocks).unwrap(), theirs);
    }

    #[test]
    fn append_at_end_both_sides() {
        // the scenario from the live test: both sides appended a line at the end, without final newline
        let base = "# J\n- a";
        let mine = "# J\n- a local";
        let theirs = "# J\n- a\n- distant";
        let mut blocks = build(base, mine, theirs);
        let (total, _) = conflict_count(&blocks);
        assert_eq!(total, 1);
        for b in &mut blocks {
            if let Block::Conflict { choice, .. } = b {
                *choice = Choice::MineThenTheirs;
            }
        }
        assert_eq!(assemble(&blocks).unwrap(), "# J\n- a local\n- a\n- distant");
    }

    #[test]
    fn unified_diff() {
        let hunks = unified("a\nb\nc\n", "a\nB\nc\n", 1);
        assert_eq!(hunks.len(), 1);
        let tags: Vec<_> = hunks[0].iter().map(|l| l.tag).collect();
        assert_eq!(tags, vec![ChangeTag::Equal, ChangeTag::Delete, ChangeTag::Insert, ChangeTag::Equal]);
    }
}
