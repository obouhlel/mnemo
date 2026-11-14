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

/// Position du curseur (en caractères) après le remplacement de `old` par `new` (texte arrivé d'un autre
/// appareil, fusion) : le curseur reste sur la même ligne, à la même colonne, même si des lignes ont été
/// ajoutées ou retirées au-dessus. Si sa ligne a elle-même été remplacée, il va sur la ligne qui la remplace.
pub fn remap_cursor(old: &str, new: &str, pos: usize) -> usize {
    // ligne et colonne du curseur dans l'ancien texte
    let before: String = old.chars().take(pos).collect();
    let line = before.matches('\n').count();
    let col = before.chars().rev().take_while(|c| *c != '\n').count();
    let diff = TextDiff::from_lines(old, new);
    let mut new_line = new.split('\n').count().saturating_sub(1);
    for op in diff.ops() {
        let (o, n) = (op.old_range(), op.new_range());
        if line < o.start {
            break;
        }
        if o.contains(&line) || (o.is_empty() && o.start == line) {
            new_line = match op.tag() {
                similar::DiffTag::Equal => n.start + (line - o.start),
                _ if n.is_empty() => n.start,
                _ => (n.start + (line - o.start)).min(n.end - 1),
            };
            if o.contains(&line) {
                break;
            }
        }
    }
    let mut idx = 0;
    for (i, l) in new.split('\n').enumerate() {
        let len = l.chars().count();
        if i == new_line {
            return idx + col.min(len);
        }
        idx += len + 1;
    }
    new.chars().count()
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
    fn cursor_follows_its_line() {
        let old = "un\ndeux\ntrois";
        let pos = "un\ndeux\ntro".chars().count();
        // une ligne allongée et une ligne ajoutée au-dessus : même ligne, même colonne
        let new = "un et demi\nnouvelle\ndeux\ntrois";
        assert_eq!(remap_cursor(old, new, pos), "un et demi\nnouvelle\ndeux\ntro".chars().count());
        // lignes retirées au-dessus
        assert_eq!(remap_cursor(old, "trois", pos), 3);
        // rien n'a changé avant le curseur
        assert_eq!(remap_cursor(old, "un\ndeux\ntrois\nquatre", pos), pos);
        // la ligne du curseur est remplacée : il reste dessus, colonne bornée
        assert_eq!(remap_cursor("a\nbcdef", "a\nXY", 6), 4);
        assert_eq!(remap_cursor("", "abc", 0), 0);
        assert_eq!(remap_cursor("é\nà", "x\né\nà", 3), 5);
    }

    #[test]
    fn unified_diff() {
        let hunks = unified("a\nb\nc\n", "a\nB\nc\n", 1);
        assert_eq!(hunks.len(), 1);
        let tags: Vec<_> = hunks[0].iter().map(|l| l.tag).collect();
        assert_eq!(tags, vec![ChangeTag::Equal, ChangeTag::Delete, ChangeTag::Insert, ChangeTag::Equal]);
    }
}
