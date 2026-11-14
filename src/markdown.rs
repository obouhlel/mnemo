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
    let mut fence = Fence::default();
    for line in src.split_inclusive('\n') {
        // Bloc de code (barrière, ou indenté de 4 espaces / tabulation) : laissé tel quel
        if fence.code_line(line) || line.starts_with("    ") || line.starts_with('\t') {
            out.push_str(line);
            continue;
        }
        transform_line(line, &mut out, resolve);
    }
    out
}

/// Suivi des blocs de code à barrière, selon CommonMark (mêmes règles que `local::index::mask_code`) :
/// une barrière s'ouvre par au moins 3 `` ` `` ou 3 `~` (indentation de 3 espaces au plus ; après des `` ` ``,
/// le reste de la ligne ne contient pas de `` ` `` : "```code```" est du code en ligne) et ne se ferme que par
/// une ligne du même caractère, au moins aussi longue, sans rien d'autre.
#[derive(Default)]
struct Fence(Option<(char, usize)>);

impl Fence {
    /// Vrai si la ligne fait partie d'un bloc de code (barrières comprises).
    fn code_line(&mut self, line: &str) -> bool {
        let t = line.trim_start_matches(' ');
        let indent = line.len() - t.len();
        let mark = t.chars().next().filter(|c| matches!(c, '`' | '~'));
        let run = mark.map(|m| t.chars().take_while(|c| *c == m).count()).unwrap_or(0);
        let after = &t[run..];
        match self.0 {
            Some((m, n)) => {
                if mark == Some(m) && run >= n && indent <= 3 && after.trim().is_empty() {
                    self.0 = None;
                }
                true
            }
            None if indent <= 3 && run >= 3 && !(mark == Some('`') && after.contains('`')) => {
                self.0 = mark.map(|m| (m, run));
                true
            }
            None => false,
        }
    }
}

/// Fin (exclue) du code en ligne ouvert par `n` accents graves en `i` : la suite fermante a exactement `n` accents.
fn inline_code_end(bytes: &[u8], i: usize, n: usize) -> Option<usize> {
    let mut j = i + n;
    while j < bytes.len() {
        if bytes[j] == b'`' {
            let m = bytes[j..].iter().take_while(|b| **b == b'`').count();
            if m == n {
                return Some(j + m);
            }
            j += m;
        } else {
            j += 1;
        }
    }
    None
}

/// Longueur en octets du tag qui commence à `rest` (juste après `#`), 0 s'il n'y en a pas. Règle d'Obsidian,
/// la même que `local::index::tags` : lettres, chiffres, `_`, `-`, `/`, avec au moins un caractère qui n'est
/// pas un chiffre (`#123` est un numéro d'issue, pas un tag).
fn tag_len(rest: &str) -> usize {
    let len: usize =
        rest.chars().take_while(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '/')).map(char::len_utf8).sum();
    if rest[..len].chars().all(|c| c.is_ascii_digit()) { 0 } else { len }
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
                // code en ligne : jusqu'à la suite du même nombre d'accents graves
                let n = bytes[i..].iter().take_while(|b| **b == b'`').count();
                i = inline_code_end(bytes, i, n).unwrap_or(i + n);
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
            b'#' if line[..i].chars().next_back().is_none_or(char::is_whitespace) => {
                let rest = &line[i + 1..];
                let len = tag_len(rest);
                if len > 0 {
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
    let mut fence = Fence::default();
    for (n, line) in src.lines().enumerate() {
        if fence.code_line(line) {
            continue;
        }
        let t = line.trim_start();
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
        // règle d'Obsidian : jamais purement numérique, mais un chiffre en tête est permis
        assert_eq!(preprocess("#123 #2024-11 #1abc"), "#123 [#2024-11](<mnemo:tag/2024-11>) [#1abc](<mnemo:tag/1abc>)");
        assert_eq!(preprocess("a\u{a0}#b x#c ##d"), "a\u{a0}[#b](<mnemo:tag/b>) x#c ##d");
        assert_eq!(preprocess("`#non` ``a ` #non`` #oui"), "`#non` ``a ` #non`` [#oui](<mnemo:tag/oui>)");
    }

    #[test]
    fn fences() {
        // "```code```" sur une ligne est du code en ligne : la suite reste transformée
        assert_eq!(preprocess("```code```\n[[y]] #t"), "```code```\n[y](<mnemo:note/y>) [#t](<mnemo:tag/t>)");
        // fermeture : même caractère, au moins la même longueur, rien d'autre sur la ligne
        let src = "````\n```\n[[x]]\n~~~~\n```` suite\n`````\n[[y]]";
        assert_eq!(preprocess(src), src.replace("[[y]]", "[y](<mnemo:note/y>)"));
        let src = "~~~ rust `x`\n#tag\n```\n~~~\n#tag";
        assert_eq!(preprocess(src), "~~~ rust `x`\n#tag\n```\n~~~\n[#tag](<mnemo:tag/tag>)");
        let h = headings("# A\n~~~\n# non\n```\n# non\n~~~\n```x```\n## B\n````\n# non\n```\n# non\n````\n### C");
        assert_eq!(h.iter().map(|h| h.1.as_str()).collect::<Vec<_>>(), ["A", "B", "C"]);
    }

    #[test]
    fn pending() {
        assert_eq!(pending_wikilink("abc [[No"), Some("No"));
        assert_eq!(pending_wikilink("abc [[No]] d"), None);
        assert_eq!(headings("# A\n```\n# no\n```\n## B ##")[1].1, "B");
    }
}
