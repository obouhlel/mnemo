//! Markdown helpers: converts `[[wikilinks]]` and `#tags` into regular CommonMark links
//! with a custom `mnemo:` scheme, so the viewer renders them as clickable links that the app
//! intercepts. Code blocks and inline code are left untouched.

pub const NOTE_SCHEME: &str = "mnemo:note/";
pub const TAG_SCHEME: &str = "mnemo:tag/";
pub const BOARD_SCHEME: &str = "mnemo:board/";
/// pièce jointe ouverte dans Mnemo (visionneuse d'images, diagramme Mermaid `.mmd`)
pub const FILE_SCHEME: &str = "mnemo:file/";

/// Ce que désigne un fichier intégré `![[…]]` ou `![](…)` (résolu par l'appelant dans le dossier des notes).
pub enum Embed {
    /// image affichable : adresse à charger (`file:///…`)
    Image(String),
    /// whiteboard : chemin relatif du fichier `.mnemo-board`
    Board(String),
    /// autre fichier (PDF, son…) : adresse ouverte avec l'application du système
    File(String),
    /// fichier ouvert dans Mnemo au clic (chemin relatif) ; intégré, il est affiché comme l'image `uri`
    Native { rel: String, uri: Option<String> },
}

/// Extensions d'images affichées dans les notes.
pub const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg"];

pub fn is_image(path: &str) -> bool {
    path.rsplit_once('.').is_some_and(|(_, e)| IMAGE_EXTS.contains(&e.to_lowercase().as_str()))
}

/// Extensions des pièces jointes (fichiers qui ne sont pas des notes) reconnues dans les liens : `[[x.pdf]]`
/// ou `![[x.png]]` désigne un fichier, jamais une note à créer, et n'est pas compté comme lien cassé.
pub const FILE_EXTS: &[&str] = &[
    "png",
    "jpg",
    "jpeg",
    "gif",
    "webp",
    "bmp",
    "svg",
    "avif",
    "ico",
    "tif",
    "tiff",
    "pdf",
    "mp3",
    "wav",
    "ogg",
    "m4a",
    "flac",
    "webm",
    "mp4",
    "mov",
    "mkv",
    "ogv",
    "3gp",
    "zip",
    "mmd",
    "canvas",
    "mnemo-board",
    "excalidraw",
    "csv",
    "xlsx",
    "docx",
    "pptx",
    "odt",
    "txt",
    "json",
    "html",
];

/// Vrai si la cible d'un lien désigne une pièce jointe (voir `FILE_EXTS`), pas une note.
pub fn is_attachment(target: &str) -> bool {
    let name = target.rsplit('/').next().unwrap_or(target);
    name.rsplit_once('.').is_some_and(|(stem, e)| !stem.is_empty() && FILE_EXTS.contains(&e.to_lowercase().as_str()))
}

/// Largeur demandée par `![[image.png|300]]` ou `![[image.png|300x200]]` (Obsidian) : la partie après `|`.
fn embed_width(spec: &str) -> Option<u32> {
    let w = spec.trim().split('x').next()?.trim();
    w.parse::<u32>().ok().filter(|w| (1..=10_000).contains(w))
}

pub type Resolver<'a> = &'a dyn Fn(&str) -> Option<Embed>;
/// Lien Markdown interne `[texte](Dossier/Note.md)` (cible brute) → chemin de la note à ouvrir.
pub type LinkResolver<'a> = &'a dyn Fn(&str) -> Option<String>;

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
        // lien vers un titre de la même note (`[[#Titre]]`) : le titre seul, comme Obsidian
        _ => match target_part.trim().strip_prefix('#') {
            Some(heading) => heading.trim().replace('#', " > "),
            None => target_part.trim().replace('#', " > "),
        },
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
    preprocess_links(src, resolve, &|_| None)
}

/// Comme `preprocess_with`, et les liens Markdown internes (`[texte](Dossier/Note.md)`) que `notes` résout
/// deviennent des liens `mnemo:` vers la note, au lieu d'adresses que le système ne saurait pas ouvrir.
pub fn preprocess_links(src: &str, resolve: Resolver, notes: LinkResolver) -> String {
    preprocess_full(src, resolve, notes, &|_| None)
}

/// Diagramme Mermaid (code du bloc) → adresse de l'image rendue ; `None` : bloc laissé en code.
pub type DiagramResolver<'a> = &'a dyn Fn(&str) -> Option<String>;

/// Lien qui marque un texte surligné `==texte==` (rendu sans lien par le visualiseur, voir `vendor/`).
pub const MARK_SCHEME: &str = "mnemo:mark";

/// Échappe la ponctuation Markdown d'un texte à afficher tel quel.
fn md_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_ascii_punctuation() {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Formule LaTeX → texte Unicode en italique (voir `math::convert`) ; lignes séparées par `\\`.
fn math_markdown(tex: &str) -> String {
    crate::math::convert(tex)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| format!("*{}*", md_escape(l)))
        .collect::<Vec<_>>()
        .join("  \n")
}

/// Comme `preprocess_links`, et les blocs ```` ```mermaid ```` que `diagram` sait rendre deviennent des images ;
/// formules `$…$` / `$$…$$` en texte mathématique, `==texte==` surligné.
pub fn preprocess_full(src: &str, resolve: Resolver, notes: LinkResolver, diagram: DiagramResolver) -> String {
    let mut out = String::with_capacity(src.len() + 64);
    let mut fence = Fence::default();
    // bloc Mermaid en cours : (texte d'origine, code)
    let mut mermaid: Option<(String, String)> = None;
    // formule $$ … $$ sur plusieurs lignes en cours : (texte d'origine, formule)
    let mut math: Option<(String, String)> = None;
    for line in src.split_inclusive('\n') {
        let was_open = fence.0.is_some();
        if fence.code_line(line) {
            if let Some((raw, code)) = &mut mermaid {
                raw.push_str(line);
                if fence.0.is_none() {
                    match diagram(code) {
                        Some(uri) => out.push_str(&format!("![Diagramme Mermaid](<{}>)\n", encode(&uri))),
                        None => out.push_str(raw),
                    }
                    mermaid = None;
                } else {
                    code.push_str(line);
                }
                continue;
            }
            if !was_open && line.trim_start().trim_start_matches(['`', '~']).trim().eq_ignore_ascii_case("mermaid") {
                mermaid = Some((line.to_string(), String::new()));
                continue;
            }
            out.push_str(line);
            continue;
        }
        if let Some((raw, tex)) = &mut math {
            raw.push_str(line);
            match line.trim_end().strip_suffix("$$") {
                Some(last) => {
                    tex.push_str(last);
                    out.push_str(&format!("\n{}\n\n", math_markdown(tex)));
                    math = None;
                }
                None => tex.push_str(line),
            }
            continue;
        }
        // Bloc de code indenté de 4 espaces / tabulation : laissé tel quel
        if line.starts_with("    ") || line.starts_with('\t') {
            out.push_str(line);
            continue;
        }
        if let Some(rest) = line.trim().strip_prefix("$$") {
            match rest.strip_suffix("$$") {
                Some(tex) => out.push_str(&format!("\n{}\n\n", math_markdown(tex))),
                None => math = Some((line.to_string(), format!("{rest}\n"))),
            }
            continue;
        }
        transform_line(line, &mut out, resolve, notes);
    }
    // bloc jamais fermé : texte d'origine
    if let Some((raw, _)) = mermaid.or(math) {
        out.push_str(&raw);
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

/// `![[cible]]` ou `![](cible)` d'un fichier : image Markdown ou lien vers le whiteboard. Une largeur
/// (`|300`) est passée au rendu dans le fragment de l'adresse (`#mnemo-w=300`, retiré avant le chargement).
fn embed_markdown(target: &str, label: &str, width: Option<u32>, resolve: Resolver) -> Option<String> {
    let label = label.replace('[', "\\[").replace(']', "\\]");
    match resolve(target)? {
        Embed::Image(uri) => {
            let w = width.map(|w| format!("#mnemo-w={w}")).unwrap_or_default();
            Some(format!("![{label}](<{}{w}>)", encode(&uri)))
        }
        Embed::Board(rel) => Some(format!("[🖊 {label}](<{BOARD_SCHEME}{}>)", encode(&rel))),
        Embed::File(uri) => Some(format!("[📎 {label}](<{}>)", encode(&uri))),
        Embed::Native { uri: Some(uri), .. } => {
            let w = width.map(|w| format!("#mnemo-w={w}")).unwrap_or_default();
            Some(format!("![{label}](<{}{w}>)", encode(&uri)))
        }
        Embed::Native { rel, uri: None } => Some(format!("[📎 {label}](<{FILE_SCHEME}{}>)", encode(&rel))),
    }
}

/// Pièce jointe introuvable : texte barré, sans lien (un clic ne doit pas créer une note « x.png »).
fn missing_file(target: &str) -> String {
    format!("~~{}~~ *(introuvable)*", target.replace('[', "\\[").replace(']', "\\]").replace('~', "\\~"))
}

fn transform_line(line: &str, out: &mut String, resolve: Resolver, notes: LinkResolver) {
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
                        // ![[image.png|300]] : la partie après | est une largeur, pas un libellé
                        let (file, spec) = inner.split_once('|').unwrap_or((inner, ""));
                        if embed && let Some(md) = embed_markdown(file.trim(), &target, embed_width(spec), resolve) {
                            out.push_str(&md);
                            i += 2 + end + 2;
                            last = i;
                            continue;
                        }
                        if embed && is_attachment(&target) {
                            out.push_str(&missing_file(&target));
                            i += 2 + end + 2;
                            last = i;
                            continue;
                        }
                        if is_attachment(&target) {
                            // [[document.pdf]] : ouvert avec l'application du système s'il existe
                            match resolve(&target) {
                                Some(Embed::Image(uri) | Embed::File(uri)) => out.push_str(&format!(
                                    "[{}](<{}>)",
                                    display.replace('[', "\\[").replace(']', "\\]"),
                                    encode(&uri)
                                )),
                                Some(Embed::Board(rel)) => {
                                    out.push_str(&format!("[🖊 {display}](<{BOARD_SCHEME}{}>)", encode(&rel)))
                                }
                                Some(Embed::Native { rel, .. }) => {
                                    out.push_str(&format!("[{display}](<{FILE_SCHEME}{}>)", encode(&rel)))
                                }
                                None => out.push_str(&missing_file(&target)),
                            }
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
            // $formule$ (et $$formule$$ dans une ligne) : pas « 5 $ et 10 $ » (règle d'Obsidian)
            b'$' if i == 0 || bytes[i - 1] != b'\\' => {
                let n = if bytes.get(i + 1) == Some(&b'$') { 2 } else { 1 };
                let body = &line[i + n..];
                let close = if n == 2 {
                    body.find("$$")
                } else {
                    body.char_indices().filter(|(k, c)| *c == '$' && *k > 0).map(|(k, _)| k).find(|&k| {
                        !body[..k].ends_with(char::is_whitespace)
                            && !body[..k].ends_with('\\')
                            && !body[k + 1..].starts_with(|c: char| c.is_ascii_digit())
                    })
                };
                match close {
                    Some(k) if k > 0 && !body.starts_with(char::is_whitespace) => {
                        out.push_str(&line[last..i]);
                        out.push_str(&math_markdown(&body[..k]));
                        i += n + k + n;
                        last = i;
                        continue;
                    }
                    _ => i += n,
                }
            }
            // ==texte surligné==
            b'=' if bytes.get(i + 1) == Some(&b'=')
                && (i == 0 || bytes[i - 1] != b'=')
                && bytes.get(i + 2).is_some_and(|c| *c != b'=' && !c.is_ascii_whitespace()) =>
            {
                let body = &line[i + 2..];
                let close = body
                    .match_indices("==")
                    .map(|(k, _)| k)
                    .find(|&k| !body[..k].ends_with(char::is_whitespace) && !body[k + 2..].starts_with('='));
                match close {
                    Some(k) if !body[..k].contains(['[', ']', '\n']) => {
                        out.push_str(&line[last..i]);
                        out.push_str(&format!("[{}](<{MARK_SCHEME}>)", &body[..k]));
                        i += 2 + k + 2;
                        last = i;
                        continue;
                    }
                    _ => i += 2,
                }
            }
            // [texte](Dossier/Note.md) : lien Markdown interne vers une note
            b'[' if (i == 0 || bytes[i - 1] != b'!') && bytes.get(i + 1) != Some(&b'[') => {
                let rest = &line[i + 1..];
                if let Some(close) = rest.find("](")
                    && !rest[..close].contains(['[', ']'])
                    && let Some(end) = rest[close + 2..].find(')')
                    && let raw = rest[close + 2..close + 2 + end].trim()
                    && let Some(path) = notes(raw.trim_start_matches('<').trim_end_matches('>'))
                {
                    out.push_str(&line[last..i]);
                    out.push_str(&format!("[{}](<{NOTE_SCHEME}{}>)", &rest[..close], encode(&path)));
                    i += 1 + close + 2 + end + 1;
                    last = i;
                    continue;
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
                        // ![légende|300](image.png) : largeur après | dans le texte, comme Obsidian
                        let (label, width) = match rest[..close].rsplit_once('|') {
                            Some((l, w)) if embed_width(w).is_some() => (l, embed_width(w)),
                            _ => (&rest[..close], None),
                        };
                        if !external && let Some(md) = embed_markdown(&target, label, width, resolve) {
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

/// Propriété du frontmatter YAML d'une note (comme le panneau « Propriétés » d'Obsidian).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Property {
    pub key: String,
    pub values: Vec<String>,
    /// valeur en liste (`[a, b]` ou lignes `- a`) : affichée en pastilles
    pub list: bool,
}

/// Frontmatter en tête de note (`---` … `---`) : ses propriétés et l'octet où commence le reste de la note.
/// Couvre ce qu'écrit Obsidian : valeurs simples, listes `[a, b]` et listes en lignes `- a` ; une valeur plus
/// complexe (dictionnaire imbriqué, texte sur plusieurs lignes) est montrée telle quelle.
pub fn frontmatter(src: &str) -> Option<(Vec<Property>, usize)> {
    let first = src.split_inclusive('\n').next()?;
    if first.trim_end() != "---" {
        return None;
    }
    let mut offset = first.len();
    let mut props: Vec<Property> = vec![];
    for line in src[offset..].split_inclusive('\n') {
        offset += line.len();
        let t = line.trim_end();
        if t == "---" || t == "..." {
            return Some((props, offset));
        }
        if t.trim().is_empty() || t.trim_start().starts_with('#') {
            continue;
        }
        let indented = t.starts_with([' ', '\t']);
        if let (true, Some(item)) = (indented || t.starts_with("- "), t.trim_start().strip_prefix('-'))
            && let Some(last) = props.last_mut()
        {
            last.list = true;
            let v = unquote(item.trim());
            if !v.is_empty() {
                last.values.push(v);
            }
            continue;
        }
        if indented {
            // suite d'une valeur complexe : gardée telle quelle
            if let Some(last) = props.last_mut() {
                match last.values.last_mut() {
                    Some(v) if !last.list => {
                        v.push(' ');
                        v.push_str(t.trim());
                    }
                    _ => last.values.push(t.trim().to_string()),
                }
            }
            continue;
        }
        let Some((key, value)) = t.split_once(':') else { continue };
        let value = value.trim();
        let (values, list) = match value.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
            Some(inner) => (inner.split(',').map(|v| unquote(v.trim())).filter(|v| !v.is_empty()).collect(), true),
            None if value.is_empty() || value == "|" || value == ">" => (vec![], false),
            None => (vec![unquote(value)], false),
        };
        props.push(Property { key: unquote(key.trim()), values, list });
    }
    None // pas de fin : ce n'est pas un frontmatter
}

/// (début, fin, liste en ligne `[a, b]`) des lignes d'une propriété.
type PropSpan = (usize, usize, bool);

/// Lignes de la propriété `key` dans le frontmatter : (début, fin) en octets, valeur en liste `[a, b]` sur la
/// ligne de la clé ; et l'octet de la ligne `---` fermante.
fn property_span(src: &str, key: &str) -> Option<(Option<PropSpan>, usize)> {
    let first = src.split_inclusive('\n').next()?;
    if first.trim_end() != "---" {
        return None;
    }
    let mut offset = first.len();
    let mut span: Option<(usize, usize, bool)> = None;
    for line in src[offset..].split_inclusive('\n') {
        let t = line.trim_end();
        if t == "---" || t == "..." {
            return Some((span, offset));
        }
        let continuation = t.starts_with([' ', '\t']) || t.starts_with("- ") || t == "-" || t.trim().is_empty();
        match (&mut span, continuation) {
            (Some((_, end, _)), true) if *end == offset => *end = offset + line.len(),
            (None, false) => {
                if let Some((k, v)) = t.split_once(':')
                    && unquote(k.trim()) == key
                {
                    span = Some((offset, offset + line.len(), v.trim().starts_with('[')));
                }
            }
            _ => {}
        }
        offset += line.len();
    }
    None
}

/// Valeur YAML simple, entre guillemets si elle serait mal relue telle quelle.
fn yaml_scalar(v: &str) -> String {
    let special = v.is_empty()
        || v != v.trim()
        || v.contains(": ")
        || v.contains(" #")
        || v.ends_with(':')
        || v.starts_with(['[', ']', '{', '}', '&', '*', '!', '|', '>', '\'', '"', '%', '@', '`', '#', ','])
        || v.starts_with("- ");
    if special { format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\"")) } else { v.to_string() }
}

/// Écrit la propriété `key` du frontmatter (créé s'il n'y en a pas) : une seule valeur (`key: v`) ou une liste,
/// au style de la propriété existante (`[a, b]` ou lignes `- a`). Le reste du texte est gardé à l'identique,
/// pour ne pas créer de faux changements avec le coffre. `values` vide et `list` faux : `key:`.
pub fn set_property(src: &str, key: &str, values: &[String], list: bool) -> String {
    let render = |inline: bool| {
        let k = yaml_scalar(key);
        if !list {
            return match values.first() {
                Some(v) if !v.is_empty() => format!("{k}: {}\n", yaml_scalar(v)),
                _ => format!("{k}:\n"),
            };
        }
        if inline {
            return format!("{k}: [{}]\n", values.iter().map(|v| yaml_scalar(v)).collect::<Vec<_>>().join(", "));
        }
        let mut out = format!("{k}:\n");
        for v in values {
            out.push_str(&format!("  - {}\n", yaml_scalar(v)));
        }
        out
    };
    match property_span(src, key) {
        Some((Some((a, b, inline)), _)) => {
            let mut line = render(inline);
            if !src[..b].ends_with('\n') {
                line.pop();
            }
            format!("{}{line}{}", &src[..a], &src[b..])
        }
        Some((None, close)) => format!("{}{}{}", &src[..close], render(false), &src[close..]),
        None => format!("---\n{}---\n{src}", render(false)),
    }
}

/// Retire la propriété `key` du frontmatter.
pub fn remove_property(src: &str, key: &str) -> String {
    match property_span(src, key) {
        Some((Some((a, b, _)), _)) => format!("{}{}", &src[..a], &src[b..]),
        _ => src.to_string(),
    }
}

/// Valeurs des propriétés `keys` du frontmatter (`tags`, `aliases`…), listes ou texte simple. Une valeur simple
/// peut en contenir plusieurs, séparées par des virgules (`tags: a, b`, accepté par Obsidian).
pub fn frontmatter_values(src: &str, keys: &[&str]) -> Vec<String> {
    let Some((props, _)) = frontmatter(src) else { return vec![] };
    let mut out: Vec<String> = vec![];
    for p in props.iter().filter(|p| keys.iter().any(|k| p.key.eq_ignore_ascii_case(k))) {
        for v in &p.values {
            let parts: Vec<&str> = if p.list { vec![v.as_str()] } else { v.split(',').collect() };
            for part in parts.into_iter().map(str::trim).filter(|v| !v.is_empty() && *v != "null") {
                if !out.iter().any(|o| o == part) {
                    out.push(part.to_string());
                }
            }
        }
    }
    out
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    for q in ['"', '\''] {
        if s.len() >= 2 && s.starts_with(q) && s.ends_with(q) {
            return s[1..s.len() - 1].to_string();
        }
    }
    s.to_string()
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
        assert_eq!(p("![légende](img/photo.png)"), "![légende](<file:///v/img/photo.png>)");
        assert_eq!(p("![x](https://exemple.fr/a.png)"), "![x](https://exemple.fr/a.png)");
        assert_eq!(p("![[croquis.mnemo-board]]"), "[🖊 croquis.mnemo-board](<mnemo:board/Dessins/croquis.mnemo-board>)");
        // diagramme .mmd : intégré comme une image, en lien ouvert dans Mnemo (pas dans le navigateur)
        let native = |t: &str| {
            (t == "flow.mmd")
                .then(|| Embed::Native { rel: "A/flow.mmd".into(), uri: Some("bytes://mermaid/1.svg".into()) })
        };
        let n = |s: &str| preprocess_with(s, &native);
        assert_eq!(n("![[flow.mmd]]"), "![flow.mmd](<bytes://mermaid/1.svg>)");
        assert_eq!(n("[[flow.mmd]]"), "[flow.mmd](<mnemo:file/A/flow.mmd>)");
        // pièce jointe introuvable : jamais un lien vers une note à créer
        assert_eq!(p("![[absent.png]]"), "~~absent.png~~ *(introuvable)*");
        assert_eq!(p("[[absent.pdf]]"), "~~absent.pdf~~ *(introuvable)*");
        assert_eq!(p("![[Autre note]]"), "[Autre note](<mnemo:note/Autre note>)");
        // largeur à la manière d'Obsidian
        assert_eq!(p("![[photo.png|300]]"), "![photo.png](<file:///v/img/photo.png#mnemo-w=300>)");
        assert_eq!(p("![[photo.png|300x200]]"), "![photo.png](<file:///v/img/photo.png#mnemo-w=300>)");
        assert_eq!(p("![légende|250](img/photo.png)"), "![légende](<file:///v/img/photo.png#mnemo-w=250>)");
        assert!(is_attachment("Assets/schéma.PNG") && !is_attachment("v1.2") && !is_attachment(".png"));
        assert!(is_image("A/b.JPG") && !is_image("note.md"));
    }

    #[test]
    fn markdown_note_links() {
        let notes = |raw: &str| (raw.ends_with(".md")).then(|| format!("Dossier/{}", raw.trim_end_matches(".md")));
        let p = |s: &str| preprocess_links(s, &|_| None, &notes);
        assert_eq!(p("voir [la note](Note.md) !"), "voir [la note](<mnemo:note/Dossier/Note>) !");
        assert_eq!(p("[site](https://x.fr)"), "[site](https://x.fr)");
        assert_eq!(p("`[x](Note.md)`"), "`[x](Note.md)`");
    }

    #[test]
    fn math_highlight_mermaid() {
        assert_eq!(preprocess("a ==important== b"), "a [important](<mnemo:mark>) b");
        assert_eq!(preprocess("titre\n===\na == b"), "titre\n===\na == b");
        assert_eq!(preprocess("`==non==`"), "`==non==`");
        assert_eq!(preprocess("aire $\\pi r^2$ ici"), "aire *π r²* ici");
        assert_eq!(preprocess("5 $ et 10 $"), "5 $ et 10 $");
        assert_eq!(preprocess("$$\nx^2\n$$\nsuite"), "\n*x²*\n\nsuite");
        assert_eq!(preprocess("$$a_1$$\n"), "\n*a₁*\n\n");
        let d = |code: &str| code.contains("A-->B").then(|| "bytes://mermaid/1.svg".to_string());
        let p = |s: &str| preprocess_full(s, &|_| None, &|_| None, &d);
        assert_eq!(p("```mermaid\ngraph LR\nA-->B\n```\nfin"), "![Diagramme Mermaid](<bytes://mermaid/1.svg>)\nfin");
        // diagramme non pris en charge : le code reste affiché
        assert_eq!(p("```mermaid\nautre\n```\n"), "```mermaid\nautre\n```\n");
        assert_eq!(p("```mermaid\nA-->B\n"), "```mermaid\nA-->B\n");
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

    #[test]
    fn frontmatter_properties() {
        let src = "---\ntags:\n  - moc\n  - algo\ncreated: 2026-06-14\naliases: [MOC, \"Index\"]\ncssclasses:\nsource: \"[[Autre note]]\"\n---\n\n# Titre\n";
        let (props, body) = frontmatter(src).unwrap();
        assert_eq!(&src[body..], "\n# Titre\n");
        let p = |k: &str| props.iter().find(|p| p.key == k).unwrap();
        assert_eq!(p("tags").values, ["moc", "algo"]);
        assert!(p("tags").list);
        assert_eq!(p("created").values, ["2026-06-14"]);
        assert!(!p("created").list);
        assert_eq!(p("aliases").values, ["MOC", "Index"]);
        assert!(p("cssclasses").values.is_empty());
        assert_eq!(p("source").values, ["[[Autre note]]"]);
        // liste au même niveau que la clé (autre style YAML)
        let (props, _) = frontmatter("---\ntags:\n- a\n- b\n---\n").unwrap();
        assert_eq!(props[0].values, ["a", "b"]);
        assert!(frontmatter("# Pas de frontmatter\n").is_none());
        assert!(frontmatter("---\nnon fermé\n").is_none());
        assert!(frontmatter("---\n---\ntexte").is_some_and(|(p, b)| p.is_empty() && b == 8));
    }

    #[test]
    fn frontmatter_lists() {
        let src = "---\ntags: [moc, \"#algo\"]\naliases: Index, Sommaire\nTag: x\n---\n";
        assert_eq!(frontmatter_values(src, &["tags", "tag"]), ["moc", "#algo", "x"]);
        assert_eq!(frontmatter_values(src, &["aliases"]), ["Index", "Sommaire"]);
        assert!(frontmatter_values("pas de frontmatter", &["tags"]).is_empty());
    }

    #[test]
    fn edit_properties() {
        let src = "---\ntags:\n  - a\naliases: [X]\ncreated: 2026-01-01\n---\n# Corps\n";
        let s = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            set_property(src, "tags", &s(&["a", "b"]), true),
            "---\ntags:\n  - a\n  - b\naliases: [X]\ncreated: 2026-01-01\n---\n# Corps\n"
        );
        assert_eq!(
            set_property(src, "aliases", &s(&["X", "Y: z"]), true),
            "---\ntags:\n  - a\naliases: [X, \"Y: z\"]\ncreated: 2026-01-01\n---\n# Corps\n"
        );
        assert_eq!(set_property(src, "created", &s(&["2026-02-02"]), false), src.replace("2026-01-01", "2026-02-02"));
        assert_eq!(
            set_property(src, "fait", &s(&["true"]), false),
            src.replace("---\n# Corps", "fait: true\n---\n# Corps")
        );
        assert_eq!(remove_property(src, "tags"), "---\naliases: [X]\ncreated: 2026-01-01\n---\n# Corps\n");
        assert_eq!(set_property("# Corps\n", "tags", &s(&["a"]), true), "---\ntags:\n  - a\n---\n# Corps\n");
        // relu à l'identique
        let out = set_property(src, "titre", &s(&["a: b #c"]), false);
        let (props, _) = frontmatter(&out).unwrap();
        assert_eq!(props.iter().find(|p| p.key == "titre").unwrap().values, ["a: b #c"]);
    }

    #[test]
    fn heading_link_display() {
        assert_eq!(parse_wikilink("#🖥️ Prérequis").1, "🖥️ Prérequis");
        assert_eq!(parse_wikilink("Note#Titre").1, "Note > Titre");
        assert_eq!(parse_wikilink("#Titre|Alias").1, "Alias");
    }
}
