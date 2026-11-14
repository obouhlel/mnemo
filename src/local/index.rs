//! Fonctions pures du stockage local : liens `[[…]]`, résolution, tags, recherche, rétroliens et graphe.
//! Elles reproduisent ce que faisaient les requêtes SQL (triggers de liens, ts_headline, regexp des tags).

use crate::model::{Backlink, GraphData, SearchHit, basename};
use uuid::Uuid;

/// Vue minimale d'une note pour les calculs d'index.
pub struct Doc<'a> {
    pub id: Uuid,
    pub path: &'a str,
    pub content: &'a str,
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

/// Cibles des liens `[[cible|alias]]` / `[[cible#titre]]` d'un texte (sans doublon, ordre d'apparition).
pub fn links(content: &str) -> Vec<String> {
    let masked = mask_code(content);
    let mut out: Vec<String> = vec![];
    for (a, b) in link_spans(&masked) {
        let target = content[a..b].trim();
        if !target.is_empty() && !out.iter().any(|t| t == target) {
            out.push(target.to_string());
        }
    }
    out
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

/// Note désignée par un lien (voir `resolve_in`).
pub fn resolve<'a>(docs: &'a [Doc<'a>], link: &str) -> Option<&'a Doc<'a>> {
    let id = resolve_in(docs.iter().map(|d| (d.id, d.path)), link)?;
    docs.iter().find(|d| d.id == id)
}

/// Tags `#tag` d'un texte, en minuscules, hors du code. Règle d'Obsidian : `#` en début de ligne ou après un
/// espace, suivi de lettres, chiffres, `_`, `-`, `/`, avec au moins un caractère qui n'est pas un chiffre
/// (`#123` est un numéro, pas un tag).
pub fn tags(content: &str) -> Vec<String> {
    let chars: Vec<char> = mask_code(content).chars().collect();
    let mut out: Vec<String> = vec![];
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
    let points = |text: &str| links(text).iter().any(|l| resolve(docs, l).is_some_and(|d| d.id == target.id));
    let mut out: Vec<Backlink> = docs
        .iter()
        .filter(|d| d.id != target.id && points(d.content))
        .map(|d| Backlink {
            id: d.id,
            path: d.path.to_string(),
            snippets: d
                .content
                .lines()
                .filter(|line| points(line))
                .take(3)
                .map(|l| l.trim().chars().take(200).collect())
                .collect(),
        })
        .collect();
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
        for l in links(d.content) {
            match resolve(docs, &l) {
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
    docs.iter()
        .filter_map(|d| {
            let next = rewrite_links(d.content, |name| before(name) == Some(id), target);
            (next != d.content).then_some((d.id, next))
        })
        .collect()
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
        Doc { id: Uuid::from_u128(n), path, content }
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
