//! Fonctions pures du stockage local : liens `[[…]]`, résolution, tags, recherche, rétroliens et graphe.
//! Elles reproduisent ce que faisaient les requêtes SQL (triggers de liens, ts_headline, regexp des tags).

use crate::markdown::{frontmatter, frontmatter_values, is_attachment};
use crate::model::{Backlink, GraphData, SearchHit, basename, parent};
use uuid::Uuid;

/// Vue minimale d'une note pour les calculs d'index.
pub struct Doc<'a> {
    pub id: Uuid,
    pub path: &'a str,
    pub content: &'a str,
    /// autres noms de la note (`aliases:` du frontmatter), acceptés par la résolution des `[[liens]]`
    pub aliases: Vec<String>,
}

impl<'a> Doc<'a> {
    pub fn new(id: Uuid, path: &'a str, content: &'a str) -> Self {
        Doc { id, path, content, aliases: frontmatter_values(content, &["aliases", "alias"]) }
    }
}

/// Lien sortant d'une note : `[[cible]]` (wikilien) ou `[texte](Dossier/Note.md)` (lien Markdown interne,
/// relatif au dossier de la note ou au coffre).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub target: String,
    pub md: bool,
}

/// Copie du texte où le code (blocs ``` / ~~~ et `code en ligne`) est remplacé par des espaces, octet pour
/// octet : les positions restent celles du texte d'origine. Un lien ou un tag écrit dans du code n'en est pas un.
pub fn mask_code(content: &str) -> String {
    let blank = |s: &str, out: &mut String| {
        for c in s.chars() {
            if c == '\n' {
                out.push('\n');
            } else {
                out.extend(std::iter::repeat_n(' ', c.len_utf8()));
            }
        }
    };
    let mut out = String::with_capacity(content.len());
    // bloc ouvert : (caractère, longueur de la barrière)
    let mut fence: Option<(char, usize)> = None;
    for line in content.split_inclusive('\n') {
        let t = line.trim_start_matches(' ');
        let indent = line.len() - t.len();
        let mark = t.chars().next().filter(|c| matches!(c, '`' | '~'));
        let run = mark.map(|m| t.chars().take_while(|c| *c == m).count()).unwrap_or(0);
        let after = &t[run.min(t.len())..];
        match fence {
            Some((m, n)) => {
                if mark == Some(m) && run >= n && indent <= 3 && after.trim().is_empty() {
                    fence = None;
                }
                blank(line, &mut out);
            }
            None if indent <= 3 && run >= 3 && !(mark == Some('`') && after.contains('`')) => {
                fence = mark.map(|m| (m, run));
                blank(line, &mut out);
            }
            None => mask_inline(line, &mut out),
        }
    }
    out
}

/// `code en ligne` d'une ligne : une suite de n accents graves se ferme par une suite de même longueur.
fn mask_inline(line: &str, out: &mut String) {
    let b = line.as_bytes();
    let mut i = 0;
    let mut copied = 0;
    while i < b.len() {
        if b[i] != b'`' {
            i += 1;
            continue;
        }
        let n = b[i..].iter().take_while(|c| **c == b'`').count();
        // suite fermante de même longueur
        let mut j = i + n;
        let mut close = None;
        while j < b.len() {
            if b[j] == b'`' {
                let m = b[j..].iter().take_while(|c| **c == b'`').count();
                if m == n {
                    close = Some(j + m);
                    break;
                }
                j += m;
            } else {
                j += 1;
            }
        }
        match close {
            Some(end) => {
                out.push_str(&line[copied..i]);
                out.extend(std::iter::repeat_n(' ', end - i));
                copied = end;
                i = end;
            }
            None => i += n,
        }
    }
    out.push_str(&line[copied..]);
}

/// Liens `[[cible|alias]]` / `[[cible#titre]]` d'un texte déjà masqué : (début de la cible, fin de la cible)
/// en octets, la cible étant ce qui précède `|`, `#` ou `]]`.
fn link_spans(masked: &str) -> Vec<(usize, usize)> {
    let mut out = vec![];
    let mut from = 0;
    while let Some(pos) = masked[from..].find("[[") {
        let start = from + pos + 2;
        let after = &masked[start..];
        let Some(end) = after.find("]]") else { break };
        let inner = &after[..end];
        if !inner.contains('\n') && !inner.contains(']') && !inner.contains("[[") {
            let target = inner.find(['|', '#']).unwrap_or(inner.len());
            out.push((start, start + target));
            from = start + end + 2;
        } else {
            from = start;
        }
    }
    out
}

/// Décodage `%XX` d'une adresse (UTF-8) ; une séquence invalide est gardée telle quelle.
pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

/// Liens Markdown internes `[texte](cible)` d'un texte déjà masqué : (début, fin) de la cible brute en octets
/// (sans `<…>`, sans `#titre` ni titre entre guillemets). Les images `![…](…)`, adresses externes
/// (`https:`, `mailto:`…), ancres seules et pièces jointes sont ignorées.
fn md_spans(masked: &str) -> Vec<(usize, usize)> {
    let b = masked.as_bytes();
    let mut out = vec![];
    let mut i = 0;
    while let Some(pos) = masked[i..].find("](") {
        let close = i + pos;
        i = close + 2;
        // crochet ouvrant sur la même ligne, pas une image, pas un wikilien
        let line_start = masked[..close].rfind('\n').map_or(0, |n| n + 1);
        let Some(open) = masked[line_start..close].rfind('[').map(|o| line_start + o) else { continue };
        if (open > 0 && matches!(b[open - 1], b'!' | b'[')) || masked[open + 1..close].contains(']') {
            continue;
        }
        let mut start = close + 2;
        let rest = &masked[start..];
        let end = if rest.starts_with('<') {
            start += 1;
            match rest[1..].find(['>', '\n']) {
                Some(e) if rest.as_bytes()[1 + e] == b'>' => start + e,
                _ => continue,
            }
        } else {
            match rest.find([')', '\n']) {
                Some(e) if rest.as_bytes()[e] == b')' => start + e,
                _ => continue,
            }
        };
        let raw = &masked[start..end];
        // titre facultatif : [x](cible "titre")
        let raw = raw.split(" \"").next().unwrap_or(raw).trim_end();
        let raw = raw.split('#').next().unwrap_or("");
        let t = raw.trim();
        if t.is_empty()
            || t.contains("://")
            || t.contains(' ') && !masked[close + 2..].starts_with('<')
            || t.split_once(':').is_some_and(|(scheme, _)| !scheme.contains('/') && scheme.len() > 1)
            || is_attachment(&percent_decode(t))
        {
            continue;
        }
        let a = start + (raw.len() - raw.trim_start().len());
        out.push((a, a + t.len()));
        i = end;
    }
    out
}

/// Cible normalisée d'un lien Markdown : décodée, sans `./` ni `.md`.
fn md_target(raw: &str) -> String {
    let t = percent_decode(raw.trim());
    let t = t.trim_start_matches("./");
    let t = t.strip_suffix(".md").or_else(|| t.strip_suffix(".MD")).unwrap_or(t);
    t.to_string()
}

/// Liens sortants d'un texte, wikiliens puis liens Markdown internes (sans doublon). Une pièce jointe
/// (`[[schéma.png]]`) n'est pas un lien vers une note : elle n'est pas comptée.
pub fn link_refs(content: &str) -> Vec<Link> {
    let masked = mask_code(content);
    let mut out: Vec<Link> = vec![];
    let mut push = |target: String, md: bool| {
        let l = Link { target, md };
        if !l.target.is_empty() && !out.contains(&l) {
            out.push(l);
        }
    };
    for (a, b) in link_spans(&masked) {
        let target = content[a..b].trim();
        if !is_attachment(target) {
            push(target.to_string(), false);
        }
    }
    for (a, b) in md_spans(&masked) {
        push(md_target(&content[a..b]), true);
    }
    out
}

/// Cibles des liens `[[cible|alias]]` / `[[cible#titre]]` et `[texte](cible.md)` d'un texte (sans doublon,
/// ordre d'apparition).
#[cfg_attr(not(test), allow(dead_code))]
pub fn links(content: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for l in link_refs(content) {
        if !out.contains(&l.target) {
            out.push(l.target);
        }
    }
    out
}

/// Chemin `rel` (avec `..` et `.`) vu depuis le dossier `dir`, normalisé ; `None` s'il sort du coffre.
fn join_rel(dir: Option<&str>, rel: &str) -> Option<String> {
    let mut parts: Vec<&str> = if rel.starts_with('/') { vec![] } else { dir.unwrap_or("").split('/').collect() };
    parts.retain(|p| !p.is_empty());
    for seg in rel.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    Some(parts.join("/"))
}

/// Clé de comparaison d'un lien ou d'un chemin : sans `.md`, en minuscules.
fn link_key(link: &str) -> String {
    let l = link.trim();
    l.strip_suffix(".md").or_else(|| l.strip_suffix(".MD")).unwrap_or(l).to_lowercase()
}

/// Note désignée par un lien parmi des (id, chemin) : chemin exact, sinon nom de fichier (insensible à la casse) ;
/// le chemin exact est préféré, puis le plus court.
fn resolve_in<'a>(paths: impl Iterator<Item = (Uuid, &'a str)>, link: &str) -> Option<Uuid> {
    let l = link_key(link);
    paths
        .filter(|(_, p)| p.to_lowercase() == l || basename(p).to_lowercase() == l)
        .min_by_key(|(_, p)| (p.to_lowercase() != l, p.len(), p.to_string()))
        .map(|(id, _)| id)
}

/// Lien Markdown interne vu depuis la note `source` : relatif à son dossier, sinon au coffre, sinon nom seul.
fn resolve_md_in<'a>(paths: impl Iterator<Item = (Uuid, &'a str)> + Clone, source: &str, target: &str) -> Option<Uuid> {
    let exact = |p: &str| {
        let k = link_key(p);
        paths.clone().find(|(_, q)| q.to_lowercase() == k).map(|(id, _)| id)
    };
    join_rel(parent(source), target)
        .and_then(|p| exact(&p))
        .or_else(|| exact(target.trim_start_matches('/')))
        .or_else(|| resolve_in(paths.clone(), basename(target)))
}

/// Chemin de la note désignée par le lien Markdown `raw` (brut, tel qu'écrit) depuis la note `source`, parmi
/// `paths` ; sinon le chemin à créer (relatif au dossier de la note). `None` : pas un lien vers une note.
pub fn md_link_path(paths: &[&str], source: &str, raw: &str) -> Option<String> {
    let t = md_target(raw.split('#').next().unwrap_or(""));
    let scheme = t.split_once(':').is_some_and(|(sc, _)| !sc.contains('/') && sc.len() > 1);
    if t.is_empty() || t.contains("://") || scheme || is_attachment(&t) {
        return None;
    }
    let ids: Vec<(Uuid, &str)> = paths.iter().enumerate().map(|(i, p)| (Uuid::from_u128(i as u128), *p)).collect();
    match resolve_md_in(ids.iter().copied(), source, &t) {
        Some(id) => Some(paths[id.as_u128() as usize].to_string()),
        None => join_rel(parent(source), &t),
    }
}

/// Note désignée par un lien (voir `resolve_in`), puis par un alias (`aliases:` du frontmatter).
pub fn resolve<'a>(docs: &'a [Doc<'a>], link: &str) -> Option<&'a Doc<'a>> {
    match resolve_in(docs.iter().map(|d| (d.id, d.path)), link) {
        Some(id) => docs.iter().find(|d| d.id == id),
        None => {
            let l = link.trim();
            docs.iter().filter(|d| d.aliases.iter().any(|a| a.eq_ignore_ascii_case(l))).min_by_key(|d| d.path.len())
        }
    }
}

/// Note désignée par un lien sortant de la note `source`.
pub fn resolve_link<'a>(docs: &'a [Doc<'a>], source: &str, link: &Link) -> Option<&'a Doc<'a>> {
    if !link.md {
        return resolve(docs, &link.target);
    }
    let id = resolve_md_in(docs.iter().map(|d| (d.id, d.path)), source, &link.target)?;
    docs.iter().find(|d| d.id == id)
}

/// Tags `#tag` d'un texte, en minuscules, hors du code. Règle d'Obsidian : `#` en début de ligne ou après un
/// espace, suivi de lettres, chiffres, `_`, `-`, `/`, avec au moins un caractère qui n'est pas un chiffre
/// (`#123` est un numéro, pas un tag).
pub fn tags(content: &str) -> Vec<String> {
    // tags du frontmatter (`tags: [a, b]`, `tags: a`), puis ceux du texte ; le frontmatter lui-même n'est pas
    // lu comme du texte (`couleur: #fff` n'est pas un tag)
    let mut out: Vec<String> = vec![];
    for t in frontmatter_values(content, &["tags", "tag"]) {
        for t in t.split_whitespace() {
            let t = t.trim_start_matches('#').to_lowercase();
            if !t.is_empty() && !t.chars().all(|c| c.is_ascii_digit()) && !out.contains(&t) {
                out.push(t);
            }
        }
    }
    let mut masked = mask_code(content);
    if let Some((_, body)) = frontmatter(content) {
        let blank: String = masked[..body].chars().map(|c| if c == '\n' { c } else { ' ' }).collect();
        masked.replace_range(..body, &blank);
    }
    let chars: Vec<char> = masked.chars().collect();
    for i in 0..chars.len() {
        if chars[i] != '#' || (i > 0 && !chars[i - 1].is_whitespace()) {
            continue;
        }
        let tag: String = chars[i + 1..]
            .iter()
            .take_while(|c| c.is_alphanumeric() || matches!(c, '_' | '/' | '-'))
            .collect::<String>()
            .to_lowercase();
        if tag.is_empty() || tag.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        if !out.contains(&tag) {
            out.push(tag);
        }
    }
    out
}

/// Tous les tags avec le nombre de notes qui les portent, du plus fréquent au moins fréquent.
pub fn tag_counts(docs: &[Doc]) -> Vec<(String, i64)> {
    let mut counts: std::collections::HashMap<String, i64> = Default::default();
    for d in docs {
        for t in tags(d.content) {
            *counts.entry(t).or_default() += 1;
        }
    }
    let mut v: Vec<(String, i64)> = counts.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v
}

/// Minuscule sans accent, caractère par caractère (les positions restent alignées sur le texte d'origine).
fn fold(c: char) -> char {
    let c = c.to_lowercase().next().unwrap_or(c);
    match c {
        'à' | 'â' | 'ä' | 'á' | 'ã' => 'a',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'î' | 'ï' | 'í' | 'ì' => 'i',
        'ô' | 'ö' | 'ó' | 'ò' | 'õ' => 'o',
        'ù' | 'û' | 'ü' | 'ú' => 'u',
        'ç' => 'c',
        'ÿ' => 'y',
        'ñ' => 'n',
        c => c,
    }
}

fn folded(s: &str) -> Vec<char> {
    s.chars().map(fold).collect()
}

fn find_all(hay: &[char], needle: &[char]) -> Vec<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return vec![];
    }
    (0..=hay.len() - needle.len()).filter(|&i| hay[i..i + needle.len()] == *needle).collect()
}

/// Recherche plein texte : chaque mot doit apparaître (début de mot, sans accent ni casse) dans la note ou son
/// chemin. Les extraits encadrent les correspondances de \u{1} … \u{2}, comme ts_headline.
pub fn search(docs: &[Doc], query: &str) -> Vec<SearchHit> {
    let words: Vec<Vec<char>> = query
        .split_whitespace()
        .map(|w| w.chars().filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-').collect::<String>())
        .filter(|w| !w.is_empty())
        .map(|w| folded(&w))
        .collect();
    let whole = folded(query.trim());
    if words.is_empty() && whole.is_empty() {
        return vec![];
    }
    let starts_word = |text: &[char], i: usize| i == 0 || !text[i - 1].is_alphanumeric();
    let mut hits: Vec<(f32, SearchHit)> = vec![];
    for d in docs {
        let body: Vec<char> = d.content.chars().collect();
        let fbody = folded(d.content);
        let fpath = folded(d.path);
        let path_hit = !whole.is_empty() && !find_all(&fpath, &whole).is_empty();
        let mut score = 0.0;
        let mut positions: Vec<(usize, usize)> = vec![];
        let mut all = !words.is_empty();
        for w in &words {
            let in_body: Vec<usize> = find_all(&fbody, w).into_iter().filter(|&i| starts_word(&fbody, i)).collect();
            let in_path = find_all(&fpath, w).into_iter().any(|i| starts_word(&fpath, i));
            if in_body.is_empty() && !in_path {
                all = false;
                break;
            }
            score += in_body.len().min(20) as f32 * 0.1 + if in_path { 2.0 } else { 0.0 };
            positions.extend(in_body.into_iter().map(|i| (i, i + w.len())));
        }
        if !all && !path_hit {
            continue;
        }
        if path_hit {
            score += 1.0;
        }
        positions.sort();
        hits.push((score, SearchHit { id: d.id, path: d.path.to_string(), snippet: snippet(&body, &positions) }));
    }
    hits.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.path.cmp(&b.1.path)));
    hits.into_iter().take(50).map(|h| h.1).collect()
}

/// Extrait autour de la première correspondance (environ 18 mots), correspondances marquées.
fn snippet(body: &[char], positions: &[(usize, usize)]) -> String {
    if positions.is_empty() {
        return body.iter().take(120).collect();
    }
    let first = positions[0].0;
    let start = first.saturating_sub(60);
    let start = (start..first).find(|&i| i == 0 || body[i - 1].is_whitespace()).unwrap_or(start);
    let end = (first + 100).min(body.len());
    let end = (end..body.len()).find(|&i| body[i].is_whitespace()).unwrap_or(body.len()).min(first + 140);
    let mut out = String::new();
    if start > 0 {
        out.push_str("… ");
    }
    let mut i = start;
    for &(a, b) in positions.iter().filter(|(a, b)| *a >= start && *b <= end) {
        if a < i {
            continue;
        }
        out.extend(&body[i..a]);
        out.push('\u{1}');
        out.extend(&body[a..b]);
        out.push('\u{2}');
        i = b;
    }
    out.extend(&body[i.min(end)..end]);
    if end < body.len() {
        out.push_str(" …");
    }
    out
}

/// Notes dont un lien désigne `target` (résolu comme au clic : une autre note de même nom, mais préférée par
/// la résolution, n'est pas comptée), avec jusqu'à 3 lignes d'extrait.
pub fn backlinks(docs: &[Doc], target: &Doc) -> Vec<Backlink> {
    let points = |src: &str, text: &str| {
        link_refs(text).iter().any(|l| resolve_link(docs, src, l).is_some_and(|d| d.id == target.id))
    };
    let mut out: Vec<Backlink> = docs
        .iter()
        .filter(|d| d.id != target.id && points(d.path, d.content))
        .map(|d| Backlink {
            id: d.id,
            path: d.path.to_string(),
            snippets: d
                .content
                .lines()
                .filter(|line| points(d.path, line))
                .take(3)
                .map(|l| l.trim().chars().take(200).collect())
                .collect(),
        })
        .collect();
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// Mentions non liées de `target` (comme le volet des rétroliens d'Obsidian) : notes qui écrivent son nom (ou
/// un de ses alias) en toutes lettres, mot entier, sans casse, hors code, frontmatter et liens, et qui ne la
/// lient pas déjà. Jusqu'à 3 lignes d'extrait.
pub fn unlinked_mentions(docs: &[Doc], target: &Doc) -> Vec<Backlink> {
    let mut names: Vec<String> = std::iter::once(basename(target.path).to_lowercase())
        .chain(target.aliases.iter().map(|a| a.to_lowercase()))
        .filter(|n| n.chars().count() >= 3)
        .collect();
    names.dedup();
    if names.is_empty() {
        return vec![];
    }
    let linked: Vec<Uuid> = backlinks(docs, target).into_iter().map(|b| b.id).collect();
    let mentions = |line: &str| {
        let low = line.to_lowercase();
        names.iter().any(|n| {
            low.match_indices(n.as_str()).any(|(i, _)| {
                let before = low[..i].chars().next_back();
                let after = low[i + n.len()..].chars().next();
                !before.is_some_and(|c| c.is_alphanumeric()) && !after.is_some_and(|c| c.is_alphanumeric())
            })
        })
    };
    let mut out: Vec<Backlink> = vec![];
    for d in docs.iter().filter(|d| d.id != target.id && !linked.contains(&d.id)) {
        // texte sans code, sans frontmatter ni liens (remplacés par des espaces, lignes conservées)
        let mut masked = mask_code(d.content);
        let blank = |m: &mut String, a: usize, b: usize| {
            let s: String = m[a..b].chars().map(|c| if c == '\n' { c } else { ' ' }).collect();
            m.replace_range(a..b, &s);
        };
        if let Some((_, body)) = frontmatter(d.content) {
            blank(&mut masked, 0, body);
        }
        for (a, b) in link_spans(&masked.clone()) {
            let close = masked[b..].find("]]").map_or(b, |e| b + e);
            blank(&mut masked, a, close);
        }
        let snippets: Vec<String> = masked
            .lines()
            .zip(d.content.lines())
            .filter(|(m, _)| mentions(m))
            .take(3)
            .map(|(_, l)| l.trim().chars().take(200).collect())
            .collect();
        if !snippets.is_empty() {
            out.push(Backlink { id: d.id, path: d.path.to_string(), snippets });
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// Graphe des liens ; une cible inexistante devient un nœud fantôme `ghost:<nom>`.
pub fn graph(docs: &[Doc]) -> GraphData {
    let mut g = GraphData {
        nodes: docs.iter().map(|d| (d.id.to_string(), d.path.to_string(), false)).collect(),
        edges: vec![],
    };
    let mut ghosts = std::collections::HashSet::new();
    for d in docs {
        for link in link_refs(d.content) {
            let l = link.target.clone();
            match resolve_link(docs, d.path, &link) {
                Some(t) if t.id != d.id => g.edges.push((d.id.to_string(), t.id.to_string())),
                Some(_) => {}
                None => {
                    let key = format!("ghost:{}", l.to_lowercase());
                    if ghosts.insert(key.clone()) {
                        g.nodes.push((key.clone(), l.clone(), true));
                    }
                    g.edges.push((d.id.to_string(), key));
                }
            }
        }
    }
    g
}

/// Après le renommage `old` → `new` de la note `id` (les autres notes ont leur chemin actuel dans `docs`) :
/// notes à réécrire et nouveau contenu de chacune. Seuls les liens qui désignaient cette note avant le
/// renommage sont réécrits, pas ceux d'une autre note de même nom, ni ce qui est écrit dans du code.
/// Le nom seul est utilisé dans les liens, sauf s'il désigne une autre note (alors le chemin complet).
pub fn relink(docs: &[Doc], id: Uuid, old: &str, new: &str) -> Vec<(Uuid, String)> {
    let before = |link: &str| resolve_in(docs.iter().map(|d| (d.id, if d.id == id { old } else { d.path })), link);
    let after = |link: &str| resolve_in(docs.iter().map(|d| (d.id, if d.id == id { new } else { d.path })), link);
    let target = if after(basename(new)) == Some(id) { basename(new) } else { new };
    let paths_before = || docs.iter().map(|d| (d.id, if d.id == id { old } else { d.path }));
    docs.iter()
        .filter_map(|d| {
            let next = rewrite_links(d.content, |name| before(name) == Some(id), target);
            let next = rewrite_md_links(
                &next,
                |t| resolve_md_in(paths_before(), d.path, t) == Some(id),
                |t| {
                    let src_dir = parent(d.path);
                    if join_rel(src_dir, t).is_some_and(|p| p.eq_ignore_ascii_case(old)) {
                        rel_path(src_dir.unwrap_or(""), new)
                    } else if t.trim_start_matches('/').eq_ignore_ascii_case(old) {
                        if t.starts_with('/') { format!("/{new}") } else { new.to_string() }
                    } else {
                        target.to_string()
                    }
                },
            );
            (next != d.content).then_some((d.id, next))
        })
        .collect()
}

/// Chemin de `to` vu depuis le dossier `from` (`../Autre/Note`).
fn rel_path(from: &str, to: &str) -> String {
    let a: Vec<&str> = from.split('/').filter(|s| !s.is_empty()).collect();
    let b: Vec<&str> = to.split('/').collect();
    let common = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let mut parts: Vec<&str> = vec![".."; a.len() - common];
    parts.extend(&b[common..]);
    parts.join("/")
}

/// Remplace la cible des liens Markdown internes que `matches` accepte (cible décodée, sans `.md`) par
/// `to(cible)` : `.md` et `#titre` gardés, espaces encodées `%20` hors `<…>`. Le code n'est pas touché.
fn rewrite_md_links(content: &str, matches: impl Fn(&str) -> bool, to: impl Fn(&str) -> String) -> String {
    let masked = mask_code(content);
    let mut out = String::with_capacity(content.len());
    let mut copied = 0;
    for (a, b) in md_spans(&masked) {
        let raw = &content[a..b];
        let t = md_target(raw);
        if !matches(&t) {
            continue;
        }
        let mut new = to(&t);
        if raw.to_lowercase().ends_with(".md") {
            new.push_str(".md");
        }
        let angled = a > 0 && content.as_bytes()[a - 1] == b'<';
        if !angled {
            new = new.replace('%', "%25").replace(' ', "%20");
        }
        out.push_str(&content[copied..a]);
        out.push_str(&new);
        copied = b;
    }
    out.push_str(&content[copied..]);
    out
}

/// Remplace par `to` la cible des liens `[[cible]]`, `[[cible|alias]]`, `[[cible#titre]]` que `matches` accepte
/// (alias et titre gardés). Le code n'est pas touché.
pub fn rewrite_links(content: &str, matches: impl Fn(&str) -> bool, to: &str) -> String {
    let masked = mask_code(content);
    let mut out = String::with_capacity(content.len());
    let mut copied = 0;
    for (a, b) in link_spans(&masked) {
        let name = content[a..b].trim();
        if !name.is_empty() && matches(name) {
            out.push_str(&content[copied..a]);
            out.push_str(to);
            copied = b;
        }
    }
    out.push_str(&content[copied..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc<'a>(n: u128, path: &'a str, content: &'a str) -> Doc<'a> {
        Doc::new(Uuid::from_u128(n), path, content)
    }

    #[test]
    fn markdown_links() {
        let text = "[a](Dossier/Note%20un.md) [b](<Autre note.md#Titre>) [c](https://x.fr) ![i](img.png)\n\
                    [d](mailto:a@b.c) [e](#ancre) [f](doc.pdf) `[g](Code.md)` [h](../Haut \"titre\")";
        let refs = link_refs(text);
        let md: Vec<&str> = refs.iter().filter(|l| l.md).map(|l| l.target.as_str()).collect();
        assert_eq!(md, ["Dossier/Note un", "Autre note", "../Haut"]);
        let docs = [
            doc(1, "Projets/Plan", ""),
            doc(2, "Plan", ""),
            doc(3, "Projets/Index", "[ici](Plan.md) [racine](/Plan.md) [haut](../Plan)"),
        ];
        let from = |t: &str| resolve_link(&docs, "Projets/Index", &Link { target: t.into(), md: true }).map(|d| d.id);
        assert_eq!(from("Plan"), Some(Uuid::from_u128(1)));
        assert_eq!(from("/Plan"), Some(Uuid::from_u128(2)));
        assert_eq!(from("../Plan"), Some(Uuid::from_u128(2)));
        assert_eq!(backlinks(&docs, &docs[0]).len(), 1);
        assert_eq!(backlinks(&docs, &docs[1])[0].snippets.len(), 1);
        let paths = ["Projets/Plan", "Plan"];
        assert_eq!(md_link_path(&paths, "Projets/Index", "Plan.md#x").as_deref(), Some("Projets/Plan"));
        assert_eq!(md_link_path(&paths, "Projets/Index", "../Plan").as_deref(), Some("Plan"));
        assert_eq!(md_link_path(&paths, "Projets/Index", "Neuve%20note.md").as_deref(), Some("Projets/Neuve note"));
        assert_eq!(md_link_path(&paths, "Projets/Index", "img.png"), None);
        assert_eq!(md_link_path(&paths, "Projets/Index", "mailto:a@b.c"), None);
        // renommage : le lien relatif reste relatif, `.md` et l'encodage sont gardés
        let docs = [doc(1, "Projets/Nouveau plan", ""), doc(3, "Projets/Index", "[ici](Plan.md#t) [x](<Plan>)")];
        let r = relink(&docs, Uuid::from_u128(1), "Projets/Plan", "Projets/Nouveau plan");
        assert_eq!(r[0].1, "[ici](Nouveau%20plan.md#t) [x](<Nouveau plan>)");
        let docs = [doc(1, "Archive/Plan", ""), doc(3, "Projets/Index", "[ici](Plan.md)")];
        let r = relink(&docs, Uuid::from_u128(1), "Projets/Plan", "Archive/Plan");
        assert_eq!(r[0].1, "[ici](../Archive/Plan.md)");
    }

    #[test]
    fn frontmatter_tags_and_aliases() {
        let docs = [
            doc(1, "Sommaire", "---\ntags: [MOC, \"#algo\"]\naliases:\n  - Index\ncouleur: #fff\n---\ncorps #corps"),
            doc(2, "B", "voir [[index]]"),
        ];
        assert_eq!(tags(docs[0].content), ["moc", "algo", "corps"]);
        assert_eq!(resolve(&docs, "Index").unwrap().id, Uuid::from_u128(1));
        assert_eq!(backlinks(&docs, &docs[0]).len(), 1);
    }

    #[test]
    fn attachments_are_not_links() {
        let docs = [doc(1, "A", "![[schéma.png]] [[doc.pdf|le doc]] [[Absente]]")];
        assert_eq!(links(docs[0].content), ["Absente"]);
        assert_eq!(graph(&docs).nodes.len(), 2);
    }

    #[test]
    fn unlinked() {
        let docs = [
            doc(1, "Projets/Rust", "---\naliases: [Rustlang]\n---\n"),
            doc(2, "A", "J'aime Rust.\n`Rust` en code\nrien"),
            doc(3, "B", "[[Rust]] et rust"),
            doc(4, "C", "Rusty n'est pas un mot entier\nmais rustlang oui"),
        ];
        let u = unlinked_mentions(&docs, &docs[0]);
        assert_eq!(u.iter().map(|b| b.path.as_str()).collect::<Vec<_>>(), ["A", "C"]);
        assert_eq!(u[0].snippets, ["J'aime Rust."]);
        assert_eq!(u[1].snippets, ["mais rustlang oui"]);
    }

    #[test]
    fn parse_links_and_tags() {
        assert_eq!(links("a [[B]] [[c|x]] [[Dir/D#t]] [[B]] [[ ]] [[e\n]]"), vec!["B", "c", "Dir/D"]);
        assert_eq!(tags("#a texte #Été/x-1 pas#ça ## titre\n#_b #1no #123"), vec!["a", "été/x-1", "_b", "1no"]);
    }

    #[test]
    fn resolution_graph_backlinks() {
        let docs = [doc(1, "Accueil", "voir [[Plan]] et [[Rien]]"), doc(2, "Projets/Plan", "retour [[accueil]]")];
        assert_eq!(resolve(&docs, "plan").unwrap().id, Uuid::from_u128(2));
        assert_eq!(resolve(&docs, "Projets/Plan.md").unwrap().id, Uuid::from_u128(2));
        let g = graph(&docs);
        assert_eq!(g.nodes.len(), 3); // 2 notes + le fantôme « Rien »
        assert_eq!(g.edges.len(), 3);
        let b = backlinks(&docs, &docs[1]);
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].snippets, vec!["voir [[Plan]] et [[Rien]]"]);
        assert_eq!(tag_counts(&[doc(1, "a", "#x #y"), doc(2, "b", "#x")]), vec![("x".into(), 2), ("y".into(), 1)]);
    }

    #[test]
    fn full_text_search() {
        let docs = [doc(1, "Notes/Thème", "Le thème sombre est agréable"), doc(2, "Autre", "rien à voir")];
        let hits = search(&docs, "theme agre");
        assert_eq!(hits.len(), 1);
        assert!(hits[0].snippet.contains("\u{1}thème\u{2}"));
        assert!(hits[0].snippet.contains("\u{1}agré\u{2}"));
        assert_eq!(search(&docs, "autre").len(), 1); // trouvé par le chemin
        assert!(search(&docs, "ombre").is_empty()); // dans le texte : début de mot seulement (« sombre »)
    }

    #[test]
    fn code_is_not_indexed() {
        let text = "#vrai `#faux [[Non]]` [[Oui]]\n```c\n#include <x.h>\nif [[ -z \"$x\" ]]; then\n```\n~~~\n[[Non plus]]\n~~~\nfin #fin";
        assert_eq!(tags(text), vec!["vrai", "fin"]);
        assert_eq!(links(text), vec!["Oui"]);
        // ```code``` sur une ligne : du code en ligne, pas un bloc qui avalerait la suite
        assert_eq!(links("```x``` [[A]]\n[[B]]"), vec!["A", "B"]);
        // les positions restent alignées avec des accents dans le code
        assert_eq!(mask_code("é `à` ù").len(), "é `à` ù".len());
        assert_eq!(links("`é[[x]]è` [[Été]]"), vec!["Été"]);
    }

    #[test]
    fn rename_only_touches_links_to_the_renamed_note() {
        // « Plan » à la racine et « Archive/Plan » : [[Plan]] désigne la première
        let docs = [
            doc(1, "Plan", ""),
            doc(2, "Archive/Vieux plan", ""),
            doc(3, "Index", "voir [[Plan]], [[Plan#Budget|le plan]] et [[Archive/Plan|l'ancien]]"),
        ];
        let r = relink(&docs, Uuid::from_u128(2), "Archive/Plan", "Archive/Vieux plan");
        assert_eq!(
            r,
            vec![(Uuid::from_u128(3), "voir [[Plan]], [[Plan#Budget|le plan]] et [[Vieux plan|l'ancien]]".to_string())]
        );
        // le nouveau nom est déjà celui d'une autre note, préférée par la résolution : chemin complet
        let docs = [doc(1, "Plan", ""), doc(2, "Archive/Plan", ""), doc(3, "Index", "[[Brouillon]]")];
        let r = relink(&docs, Uuid::from_u128(2), "Archive/Brouillon", "Archive/Plan");
        assert_eq!(r, vec![(Uuid::from_u128(3), "[[Archive/Plan]]".to_string())]);
    }

    #[test]
    fn rename_follows_accents_and_md_but_not_code() {
        let docs = [
            doc(1, "Lycée", ""),
            doc(2, "A", "[[école]] et [[École.md]]\n```sh\nif [[ école ]]; then\n```\n`[[École]]` [[École] x"),
        ];
        let r = relink(&docs, Uuid::from_u128(1), "École", "Lycée");
        assert_eq!(
            r,
            vec![(
                Uuid::from_u128(2),
                "[[Lycée]] et [[Lycée]]\n```sh\nif [[ école ]]; then\n```\n`[[École]]` [[École] x".to_string()
            )]
        );
        assert_eq!(
            rewrite_links(
                "a [[Old]] b [[old|alias]] c [[Dir/Old#h]] d [[Older]]",
                |n| n.to_lowercase().ends_with("old"),
                "New"
            ),
            "a [[New]] b [[New|alias]] c [[New#h]] d [[Older]]"
        );
    }

    #[test]
    fn backlinks_follow_resolution() {
        let docs = [doc(1, "Plan", ""), doc(2, "Archive/Plan", ""), doc(3, "Index", "[[Plan]]\n[[Archive/Plan]]")];
        assert_eq!(backlinks(&docs, &docs[0])[0].snippets, vec!["[[Plan]]"]);
        assert_eq!(backlinks(&docs, &docs[1])[0].snippets, vec!["[[Archive/Plan]]"]);
    }

    #[test]
    fn rename_relinks() {
        let docs = [doc(1, "Old", ""), doc(2, "A", "[[Old]] et [[old|x]]"), doc(3, "B", "rien")];
        let r = relink(&docs, Uuid::from_u128(1), "Old", "Dir/New");
        assert_eq!(r, vec![(Uuid::from_u128(2), "[[New]] et [[New|x]]".to_string())]);
    }
}
