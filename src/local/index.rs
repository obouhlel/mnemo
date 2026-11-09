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

/// Cibles des liens `[[cible|alias]]` / `[[cible#titre]]` d'un texte (sans doublon, ordre d'apparition).
pub fn links(content: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    let mut rest = content;
    while let Some(pos) = rest.find("[[") {
        let after = &rest[pos + 2..];
        let Some(end) = after.find("]]") else { break };
        let inner = &after[..end];
        if !inner.contains('\n') && !inner.contains(']') {
            let target = inner.split('|').next().unwrap_or("").split('#').next().unwrap_or("").trim();
            if !target.is_empty() && !out.iter().any(|t| t == target) {
                out.push(target.to_string());
            }
            rest = &after[end + 2..];
        } else {
            rest = after;
        }
    }
    out
}

/// Note désignée par un lien : chemin exact, sinon nom de fichier (insensible à la casse) ; le chemin exact
/// est préféré, puis le plus court.
pub fn resolve<'a>(docs: &'a [Doc<'a>], link: &str) -> Option<&'a Doc<'a>> {
    let l = link.trim().trim_end_matches(".md").to_lowercase();
    docs.iter()
        .filter(|d| d.path.to_lowercase() == l || basename(d.path).to_lowercase() == l)
        .min_by_key(|d| (d.path.to_lowercase() != l, d.path.len()))
}

/// Tags `#tag` d'un texte, en minuscules (même règle que l'ancien SQL : précédé d'un début de ligne ou d'un espace,
/// commence par une lettre ou `_`, puis lettres, chiffres, `_`, `/`, `-`).
pub fn tags(content: &str) -> Vec<String> {
    let chars: Vec<char> = content.chars().collect();
    let mut out: Vec<String> = vec![];
    for i in 0..chars.len() {
        if chars[i] != '#' || (i > 0 && !chars[i - 1].is_whitespace()) {
            continue;
        }
        let Some(&first) = chars.get(i + 1) else { continue };
        if !(first.is_alphabetic() || first == '_') {
            continue;
        }
        let tag: String = chars[i + 1..]
            .iter()
            .take_while(|c| c.is_alphanumeric() || matches!(c, '_' | '/' | '-'))
            .collect::<String>()
            .to_lowercase();
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

/// Notes qui pointent vers `target` (par son chemin ou son nom), avec jusqu'à 3 lignes d'extrait.
pub fn backlinks(docs: &[Doc], target: &Doc) -> Vec<Backlink> {
    let (pl, bl) = (target.path.to_lowercase(), basename(target.path).to_lowercase());
    let mut out: Vec<Backlink> = docs
        .iter()
        .filter(|d| d.id != target.id)
        .filter(|d| links(d.content).iter().any(|l| l.to_lowercase() == pl || l.to_lowercase() == bl))
        .map(|d| Backlink {
            id: d.id,
            path: d.path.to_string(),
            snippets: d
                .content
                .lines()
                .filter(|line| {
                    let l = line.to_lowercase();
                    [&pl, &bl].iter().any(|t| ["]]", "|", "#"].iter().any(|end| l.contains(&format!("[[{t}{end}"))))
                })
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

/// Après un renommage `old` → `new` : notes à réécrire et nouveau contenu de chacune.
/// Le nom seul est utilisé dans les liens, sauf s'il devient ambigu (alors le chemin complet).
pub fn relink(docs: &[Doc], id: Uuid, old: &str, new: &str) -> Vec<(Uuid, String)> {
    let old_base = basename(old);
    let ambiguous = docs.iter().any(|d| d.id != id && basename(d.path).eq_ignore_ascii_case(basename(new)));
    let target = if ambiguous { new } else { basename(new) };
    docs.iter()
        .filter(|d| links(d.content).iter().any(|l| l.eq_ignore_ascii_case(old) || l.eq_ignore_ascii_case(old_base)))
        .filter_map(|d| {
            let next = crate::db::rewrite_links(d.content, &[old, old_base], target);
            (next != d.content).then_some((d.id, next))
        })
        .collect()
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
        assert_eq!(tags("#a texte #Été/x-1 pas#ça ## titre\n#_b #1no"), vec!["a", "été/x-1", "_b"]);
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
    fn rename_relinks() {
        let docs = [doc(1, "Old", ""), doc(2, "A", "[[Old]] et [[old|x]]"), doc(3, "B", "rien")];
        let r = relink(&docs, Uuid::from_u128(1), "Old", "Dir/New");
        assert_eq!(r, vec![(Uuid::from_u128(2), "[[New]] et [[New|x]]".to_string())]);
    }
}
