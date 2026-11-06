//! Markdown helpers: converts `[[wikilinks]]` and `#tags` into regular CommonMark links
//! with a custom `mnemo:` scheme, so the viewer renders them as clickable links that the app
//! intercepts. Code blocks and inline code are left untouched.

pub const NOTE_SCHEME: &str = "mnemo:note/";
pub const TAG_SCHEME: &str = "mnemo:tag/";

fn encode(s: &str) -> String {
    s.replace('%', "%25").replace('<', "%3C").replace('>', "%3E")
}

pub fn decode(s: &str) -> String {
    s.replace("%3C", "<").replace("%3E", ">").replace("%25", "%")
}

/// Parses the inside of `[[...]]` into (target, display text).
pub fn parse_wikilink(inner: &str) -> (String, String) {
    let (target_part, alias) = match inner.split_once('|') {
        Some((t, a)) => (t, Some(a.trim())),
        None => (inner, None),
    };
    let target = target_part.split('#').next().unwrap_or("").trim().to_string();
    let display = match alias {
        Some(a) if !a.is_empty() => a.to_string(),
        _ => target_part.trim().replace('#', " > "),
    };
    (target, display)
}

/// Transforms wikilinks and tags of a whole document.
pub fn preprocess(src: &str) -> String {
    let mut out = String::with_capacity(src.len() + 64);
    let mut fence: Option<String> = None;
    for line in src.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if let Some(f) = &fence {
            if trimmed.starts_with(f.as_str()) {
                fence = None;
            }
            out.push_str(line);
            continue;
        }
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            let ch = trimmed.chars().next().unwrap();
            let n = trimmed.chars().take_while(|c| *c == ch).count();
            fence = Some(std::iter::repeat_n(ch, n).collect());
            out.push_str(line);
            continue;
        }
        // Indented code block (4 spaces / tab) — leave as is
        if line.starts_with("    ") || line.starts_with('\t') {
            out.push_str(line);
            continue;
        }
        transform_line(line, &mut out);
    }
    out
}

fn transform_line(line: &str, out: &mut String) {
    let bytes = line.as_bytes();
    let mut i = 0;
    let mut last = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'`' => {
                // skip inline code span with the same number of backticks
                let n = bytes[i..].iter().take_while(|b| **b == b'`').count();
                let ticks = &line[i..i + n];
                if let Some(end) = line[i + n..].find(ticks) {
                    i += n + end + n;
                } else {
                    i += n;
                }
            }
            b'[' if bytes.get(i + 1) == Some(&b'[') => {
                if let Some(end) = line[i + 2..].find("]]") {
                    let inner = &line[i + 2..i + 2 + end];
                    if !inner.is_empty() && !inner.contains('[') {
                        let embed = i > 0 && bytes[i - 1] == b'!';
                        let start = if embed { i - 1 } else { i };
                        out.push_str(&line[last..start]);
                        let (target, display) = parse_wikilink(inner);
                        out.push_str(&format!("[{}](<{}{}>)", display.replace('[', "\\[").replace(']', "\\]"), NOTE_SCHEME, encode(&target)));
                        i += 2 + end + 2;
                        last = i;
                        continue;
                    }
                }
                i += 1;
            }
            b'#' if (i == 0 || bytes[i - 1].is_ascii_whitespace()) => {
                let rest = &line[i + 1..];
                let first = rest.chars().next();
                if first.is_some_and(|c| c.is_alphabetic() || c == '_') {
                    let len: usize = rest
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '/'))
                        .map(char::len_utf8)
                        .sum();
                    let tag = &rest[..len];
                    out.push_str(&line[last..i]);
                    out.push_str(&format!("[#{tag}](<{TAG_SCHEME}{}>)", encode(tag)));
                    i += 1 + len;
                    last = i;
                    continue;
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    out.push_str(&line[last..]);
}

/// Extracts the headings of a document (level, text, line index).
#[cfg_attr(not(test), allow(dead_code))]
pub fn headings(src: &str) -> Vec<(usize, String, usize)> {
    let mut res = Vec::new();
    let mut in_fence = false;
    for (n, line) in src.lines().enumerate() {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        let level = t.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&level) && t[level..].starts_with(' ') {
            res.push((level, t[level..].trim().trim_end_matches('#').trim().to_string(), n));
        }
    }
    res
}

/// Returns the partial link text if the cursor is right after an unclosed `[[`.
pub fn pending_wikilink(text_before_cursor: &str) -> Option<&str> {
    let start = text_before_cursor.rfind("[[")?;
    let partial = &text_before_cursor[start + 2..];
    if partial.contains("]]") || partial.contains('\n') || partial.contains('|') || partial.chars().count() > 80 {
        return None;
    }
    Some(partial)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wikilinks() {
        assert_eq!(preprocess("voir [[Ma Note]] ici"), "voir [Ma Note](<mnemo:note/Ma Note>) ici");
        assert_eq!(preprocess("[[a/b#Titre|alias]]"), "[alias](<mnemo:note/a/b>)");
        assert_eq!(preprocess("![[img]]"), "[img](<mnemo:note/img>)");
        assert_eq!(preprocess("`[[pas un lien]]`"), "`[[pas un lien]]`");
        assert_eq!(preprocess("```\n[[x]]\n```\n[[y]]"), "```\n[[x]]\n```\n[y](<mnemo:note/y>)");
    }

    #[test]
    fn tags() {
        assert_eq!(preprocess("# Titre\n#projet/rust ok"), "# Titre\n[#projet/rust](<mnemo:tag/projet/rust>) ok");
        assert_eq!(preprocess("issue#12 et #42"), "issue#12 et #42");
        assert_eq!(preprocess("#été"), "[#été](<mnemo:tag/été>)");
    }

    #[test]
    fn pending() {
        assert_eq!(pending_wikilink("abc [[No"), Some("No"));
        assert_eq!(pending_wikilink("abc [[No]] d"), None);
        assert_eq!(headings("# A\n```\n# no\n```\n## B ##")[1].1, "B");
    }
}
