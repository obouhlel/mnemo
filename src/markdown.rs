//! Markdown helpers: converts `[[wikilinks]]` and `#tags` into regular CommonMark links
//! with a custom `mnemo:` scheme, so the viewer renders them as clickable links that the app
//! intercepts. Code blocks and inline code are left untouched.

pub const NOTE_SCHEME: &str = "mnemo:note/";
pub const TAG_SCHEME: &str = "mnemo:tag/";
pub const BOARD_SCHEME: &str = "mnemo:board/";

/// Ce que désigne un fichier intégré `![[…]]` ou `![](…)` (résolu par l'appelant dans le dossier des notes).
pub enum Embed {
    /// image affichable : adresse à charger (`file:///…`)
    Image(String),
    /// whiteboard : chemin relatif du fichier `.mnemo-board`
    Board(String),
}

/// Extensions d'images affichées dans les notes.
pub const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg"];

pub fn is_image(path: &str) -> bool {
    path.rsplit_once('.').is_some_and(|(_, e)| IMAGE_EXTS.contains(&e.to_lowercase().as_str()))
}

pub type Resolver<'a> = &'a dyn Fn(&str) -> Option<Embed>;

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
#[cfg_attr(not(test), allow(dead_code))]
pub fn preprocess(src: &str) -> String {
    preprocess_with(src, &|_| None)
}

/// Comme `preprocess`, et les fichiers intégrés (`![[image.png]]`, `![](dossier/image.png)`,
/// `![[croquis.mnemo-board]]`) sont résolus par `resolve` : images affichées, whiteboards en lien.
pub fn preprocess_with(src: &str, resolve: Resolver) -> String {
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
        transform_line(line, &mut out, resolve);
    }
    out
}

/// `![[cible]]` ou `![](cible)` d'un fichier : image Markdown ou lien vers le whiteboard.
fn embed_markdown(target: &str, label: &str, resolve: Resolver) -> Option<String> {
    let label = label.replace('[', "\\[").replace(']', "\\]");
    match resolve(target)? {
        Embed::Image(uri) => Some(format!("![{label}](<{}>)", encode(&uri))),
        Embed::Board(rel) => Some(format!("[🖊 {label}](<{BOARD_SCHEME}{}>)", encode(&rel))),
    }
}

fn transform_line(line: &str, out: &mut String, resolve: Resolver) {
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
                        // ![[image.png|300]] : la partie après | est une taille (ignorée), pas un libellé
                        if embed
                            && let Some(md) =
                                embed_markdown(inner.split('|').next().unwrap_or("").trim(), &target, resolve)
                        {
                            out.push_str(&md);
                            i += 2 + end + 2;
                            last = i;
                            continue;
                        }
                        out.push_str(&format!(
                            "[{}](<{}{}>)",
                            display.replace('[', "\\[").replace(']', "\\]"),
                            NOTE_SCHEME,
                            encode(&target)
                        ));
                        i += 2 + end + 2;
                        last = i;
                        continue;
                    }
                }
                i += 1;
            }
            // ![texte](chemin/relatif.png) : chemin du dossier des notes → adresse du fichier
            b'!' if bytes.get(i + 1) == Some(&b'[') && bytes.get(i + 2) != Some(&b'[') => {
                let rest = &line[i + 2..];
                if let Some(close) = rest.find("](") {
                    let after = &rest[close + 2..];
                    if let Some(end) = after.find(')') {
                        let raw = after[..end].trim().trim_start_matches('<').trim_end_matches('>');
                        let target = raw.replace("%20", " ");
                        let external = raw.contains("://") || raw.starts_with("data:");
                        if !external && let Some(md) = embed_markdown(&target, &rest[..close], resolve) {
                            out.push_str(&line[last..i]);
                            out.push_str(&md);
                            i += 2 + close + 2 + end + 1;
                            last = i;
                            continue;
                        }
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
    fn embeds() {
        let resolve = |t: &str| match t {
            "photo.png" | "img/photo.png" => Some(Embed::Image("file:///v/img/photo.png".into())),
            "croquis.mnemo-board" => Some(Embed::Board("Dessins/croquis.mnemo-board".into())),
            _ => None,
        };
        let p = |s: &str| preprocess_with(s, &resolve);
        assert_eq!(p("a ![[photo.png]] b"), "a ![photo.png](<file:///v/img/photo.png>) b");
        assert_eq!(p("![[photo.png|300]]"), "![photo.png](<file:///v/img/photo.png>)");
        assert_eq!(p("![légende](img/photo.png)"), "![légende](<file:///v/img/photo.png>)");
        assert_eq!(p("![x](https://exemple.fr/a.png)"), "![x](https://exemple.fr/a.png)");
        assert_eq!(p("![[croquis.mnemo-board]]"), "[🖊 croquis.mnemo-board](<mnemo:board/Dessins/croquis.mnemo-board>)");
        // fichier introuvable : lien vers une note, comme avant
        assert_eq!(p("![[absent.png]]"), "[absent.png](<mnemo:note/absent.png>)");
        assert!(is_image("A/b.JPG") && !is_image("note.md"));
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
