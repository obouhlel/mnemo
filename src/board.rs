//! Dashboard domain: projects, tickets with tags, reports (notes linked to tickets)
//! and the GitHub synchronisation plan (pure functions, unit-tested).

use chrono::{DateTime, Utc};
use egui::Color32;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Status {
    Todo,
    Doing,
    Blocked,
    Review,
    Done,
}

impl Status {
    pub const ALL: [Status; 5] = [Status::Todo, Status::Doing, Status::Blocked, Status::Review, Status::Done];

    pub fn as_str(self) -> &'static str {
        match self {
            Status::Todo => "todo",
            Status::Doing => "doing",
            Status::Blocked => "blocked",
            Status::Review => "review",
            Status::Done => "done",
        }
    }
    pub fn parse(s: &str) -> Status {
        match s {
            "doing" => Status::Doing,
            "blocked" => Status::Blocked,
            "review" => Status::Review,
            "done" => Status::Done,
            _ => Status::Todo,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Status::Todo => "À faire",
            Status::Doing => "En cours",
            Status::Blocked => "Bloqué",
            Status::Review => "En revue",
            Status::Done => "Terminé",
        }
    }
    pub fn color(self) -> Color32 {
        match self {
            Status::Todo => Color32::from_rgb(0x9a, 0x9a, 0x9a),
            Status::Doing => Color32::from_rgb(0x3b, 0x9e, 0xf5),
            Status::Blocked => Color32::from_rgb(0xfb, 0x46, 0x4c),
            Status::Review => Color32::from_rgb(0xe9, 0x97, 0x3f),
            Status::Done => Color32::from_rgb(0x44, 0xcf, 0x6e),
        }
    }
}

pub const PRIORITIES: [&str; 4] = ["Basse", "Normale", "Haute", "Urgente"];

pub fn priority_color(p: i16) -> Option<Color32> {
    match p {
        3 => Some(Color32::from_rgb(0xfb, 0x46, 0x4c)),
        2 => Some(Color32::from_rgb(0xe9, 0x97, 0x3f)),
        _ => None,
    }
}

/// Built-in tags: (name, colour, GitHub label, Lucide icon)
pub const TAGS: &[(&str, Color32, &str, &str)] = &[
    ("Feature", Color32::from_rgb(0x8b, 0x6c, 0xef), "feature", "\u{e417}"),
    ("Bug", Color32::from_rgb(0xfb, 0x46, 0x4c), "bug", "\u{e20c}"),
    ("Amélioration", Color32::from_rgb(0x3b, 0x9e, 0xf5), "enhancement", "\u{e1bf}"),
    ("Documentation", Color32::from_rgb(0x2e, 0xb8, 0xa6), "documentation", "\u{e54f}"),
    ("Tâche technique", Color32::from_rgb(0x9a, 0x9a, 0x9a), "chore", "\u{e1b1}"),
    ("Sécurité", Color32::from_rgb(0xe0, 0x5d, 0x9e), "security", "\u{e15c}"),
    ("Performance", Color32::from_rgb(0xe9, 0x97, 0x3f), "performance", "\u{e1bf}"),
    ("Recherche", Color32::from_rgb(0x44, 0xcf, 0x6e), "research", "\u{e0da}"),
];

/// Couleurs des labels des dépôts GitHub liés (nom en minuscules → couleur), mises à jour au chargement du tableau
/// de bord : un tag porte la même couleur que le label du dépôt.
static LABEL_COLORS: std::sync::RwLock<Option<std::collections::HashMap<String, Color32>>> =
    std::sync::RwLock::new(None);

pub fn set_label_colors(projects: &[Project]) {
    let map = projects
        .iter()
        .flat_map(|p| &p.github_labels)
        .filter_map(|(name, hex)| {
            let v = u32::from_str_radix(hex.trim_start_matches('#'), 16).ok().filter(|_| hex.len() >= 6)?;
            Some((name.to_lowercase(), Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)))
        })
        .collect();
    if let Ok(mut w) = LABEL_COLORS.write() {
        *w = Some(map);
    }
}

pub fn tag_color(tag: &str) -> Color32 {
    // couleur du label du dépôt, cherché sous le nom du tag puis sous celui de son label (« Amélioration » → enhancement)
    let repo = |name: String| LABEL_COLORS.read().ok().and_then(|m| m.as_ref().and_then(|m| m.get(&name).copied()));
    if let Some(c) = repo(tag.to_lowercase()).or_else(|| repo(tag_to_label(tag).to_lowercase())) {
        return c;
    }
    if let Some(t) = TAGS.iter().find(|t| t.0.eq_ignore_ascii_case(tag)) {
        return t.1;
    }
    // stable colour for custom tags
    let h = tag.bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ b as u32);
    let palette = [
        Color32::from_rgb(0xa8, 0x8b, 0xfa),
        Color32::from_rgb(0x5e, 0xb1, 0xef),
        Color32::from_rgb(0x4f, 0xd1, 0xa5),
        Color32::from_rgb(0xf2, 0xb8, 0x5a),
        Color32::from_rgb(0xef, 0x7b, 0x9a),
        Color32::from_rgb(0xb4, 0xc4, 0x5c),
    ];
    palette[(h as usize) % palette.len()]
}

pub fn tag_icon(tag: &str) -> &'static str {
    TAGS.iter().find(|t| t.0.eq_ignore_ascii_case(tag)).map(|t| t.3).unwrap_or("\u{e183}")
}

/// Ticket tag -> GitHub label
pub fn tag_to_label(tag: &str) -> String {
    TAGS.iter().find(|t| t.0.eq_ignore_ascii_case(tag)).map(|t| t.2.to_string()).unwrap_or_else(|| tag.to_string())
}

/// Label GitHub → tag : l'inverse de `tag_to_label` (`enhancement` → « Amélioration », `bug` → « Bug ») ;
/// un label qui n'est pas celui d'un tag intégré garde son nom.
pub fn label_to_tag(label: &str) -> String {
    TAGS.iter()
        .find(|t| t.2.eq_ignore_ascii_case(label) || t.0.eq_ignore_ascii_case(label))
        .map(|t| t.0.to_string())
        .unwrap_or_else(|| label.to_string())
}

/// Tags → labels GitHub, sans doublon (les labels GitHub ne distinguent pas la casse).
pub fn labels_from_tags(tags: &[String]) -> Vec<String> {
    let mut labels: Vec<String> = vec![];
    for l in tags.iter().map(|t| tag_to_label(t)) {
        if !labels.iter().any(|x| same_label(x, &l)) {
            labels.push(l);
        }
    }
    labels
}

fn same_label(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

fn has_label(labels: &[String], l: &str) -> bool {
    labels.iter().any(|x| same_label(x, l))
}

fn same_labels(a: &[String], b: &[String]) -> bool {
    a.iter().all(|l| has_label(b, l)) && b.iter().all(|l| has_label(a, l))
}

#[derive(Debug, Clone)]
pub struct Project {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub github_repo: Option<String>,
    /// labels du dépôt GitHub : (nom, couleur "rrggbb"), mis à jour à chaque synchronisation
    pub github_labels: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct Ticket {
    pub id: Uuid,
    pub project_id: Uuid,
    pub number: i32,
    pub title: String,
    pub body: String,
    pub status: Status,
    pub tags: Vec<String>,
    pub priority: i16,
    pub github_number: Option<i32>,
    pub github_url: Option<String>,
    pub github_state: Option<String>,
    pub github_synced_at: Option<DateTime<Utc>>,
    /// le ticket suit une pull request (et non une issue)
    pub github_pr: bool,
    /// l'issue telle que vue à la dernière synchro (None : ticket synchronisé par une ancienne version)
    pub github_base: Option<IssueBase>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// dernière modification d'un champ visible sur GitHub (titre, description, tags, ouvert / terminé)
    pub content_changed_at: Option<DateTime<Utc>>,
    pub done_at: Option<DateTime<Utc>>,
}

/// Ce que GitHub contenait à la dernière synchronisation : la base de la fusion champ par champ.
#[derive(Debug, Clone, PartialEq)]
pub struct IssueBase {
    pub title: String,
    pub body: String,
    pub labels: Vec<String>,
    pub open: bool,
    pub updated_at: DateTime<Utc>,
}

impl IssueBase {
    pub fn of(issue: &RemoteIssue) -> Self {
        IssueBase {
            title: issue.title.clone(),
            body: issue.body.clone(),
            labels: issue.labels.clone(),
            open: issue.open,
            updated_at: issue.updated_at,
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({ "title": self.title, "body": self.body, "labels": self.labels, "open": self.open,
                            "updated_at": self.updated_at.to_rfc3339() })
    }

    pub fn from_json(v: &serde_json::Value) -> Option<Self> {
        Some(IssueBase {
            title: v.get("title")?.as_str()?.to_string(),
            body: v.get("body")?.as_str()?.to_string(),
            labels: v.get("labels")?.as_array()?.iter().filter_map(|l| l.as_str().map(str::to_string)).collect(),
            open: v.get("open")?.as_bool()?,
            updated_at: DateTime::parse_from_rfc3339(v.get("updated_at")?.as_str()?).ok()?.with_timezone(&Utc),
        })
    }
}

#[derive(Debug, Clone)]
pub struct ReportLink {
    pub ticket_id: Uuid,
    pub note_id: Uuid,
    pub path: String,
}

#[derive(Debug, Clone, Default)]
pub struct Board {
    pub projects: Vec<Project>,
    pub tickets: Vec<Ticket>,
    pub reports: Vec<ReportLink>,
}

/// Fields to change on a ticket (None = unchanged)
#[derive(Debug, Clone, Default)]
pub struct TicketPatch {
    pub title: Option<String>,
    pub body: Option<String>,
    pub status: Option<Status>,
    pub tags: Option<Vec<String>>,
    pub priority: Option<i16>,
    pub project_id: Option<Uuid>,
}

/// (done, total)
pub fn progress<'a>(tickets: impl Iterator<Item = &'a Ticket>) -> (usize, usize) {
    tickets.fold((0, 0), |(d, t), k| (d + usize::from(k.status == Status::Done), t + 1))
}

// ------------------------------------------------------------------------------------ GitHub

#[derive(Debug, Clone)]
pub struct RemoteIssue {
    /// pull request (l'API des issues renvoie aussi les PR)
    pub pull: bool,
    pub draft: bool,
    pub merged: bool,
    pub number: i32,
    pub title: String,
    pub body: String,
    pub open: bool,
    pub labels: Vec<String>,
    pub url: String,
    pub updated_at: DateTime<Utc>,
}

/// Champs d'une issue à envoyer à GitHub (None : ne pas y toucher).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct IssuePatch {
    pub title: Option<String>,
    pub body: Option<String>,
    pub labels: Option<Vec<String>>,
    pub open: Option<bool>,
}

impl IssuePatch {
    pub fn is_empty(&self) -> bool {
        *self == IssuePatch::default()
    }
}

/// Résultat de la fusion d'un ticket avec son issue : les champs du ticket à modifier (None : inchangé)
/// et ceux de l'issue à envoyer.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Merged {
    pub title: Option<String>,
    pub body: Option<String>,
    pub tags: Option<Vec<String>>,
    pub status: Option<Status>,
    pub push: IssuePatch,
}

impl Merged {
    /// le ticket reçoit quelque chose de GitHub
    pub fn pulls(&self) -> bool {
        self.title.is_some() || self.body.is_some() || self.tags.is_some() || self.status.is_some()
    }
}

#[derive(Debug, Clone)]
pub enum SyncAction {
    /// Issue unknown locally: create a ticket
    Import(RemoteIssue),
    /// Ticket et issue diffèrent, ou l'issue a bougé depuis la dernière synchro : appliquer la fusion
    Update { ticket: Uuid, issue: RemoteIssue, merged: Merged },
}

/// Margin to ignore the few ms between `updated_at` and `github_synced_at` set by the same statement
const SLACK: chrono::Duration = chrono::Duration::seconds(2);

enum Side {
    Same,
    Local,
    Remote,
}

/// Quel côté garder pour un champ : celui qui a changé depuis la base ; si les deux ont changé (ou si la base
/// est inconnue), `prefer_remote` tranche.
fn pick<T: PartialEq + ?Sized>(local: &T, remote: &T, base: Option<&T>, prefer_remote: bool) -> Side {
    if local == remote {
        return Side::Same;
    }
    match base {
        Some(b) if local == b => Side::Remote,
        Some(b) if remote == b => Side::Local,
        _ if prefer_remote => Side::Remote,
        _ => Side::Local,
    }
}

fn unix_lines(s: &str) -> String {
    s.replace("\r\n", "\n")
}

/// Fusionne un ticket lié et son issue, champ par champ (titre, description, labels, ouvert / fermé), à partir
/// de ce que GitHub contenait à la dernière synchro (`github_base`) : un champ modifié d'un seul côté est repris
/// de ce côté, sans toucher aux autres. Une issue fermée sur GitHub n'est donc pas rouverte parce que le titre
/// du ticket a été corrigé. Champ modifié des deux côtés : le changement le plus récent gagne ; les labels
/// ajoutés ou retirés de chaque côté sont tous repris.
/// Sans base (ticket synchronisé par une ancienne version), les dates décident pour tous les champs qui diffèrent :
/// le côté modifié depuis la dernière synchro, sinon le plus récent.
/// Une pull request n'est jamais fermée ni rouverte depuis Mnemo : son état donne le statut du ticket.
pub fn merge_ticket(t: &Ticket, issue: &RemoteIssue) -> Merged {
    let base = t.github_base.as_ref();
    let changed = t.content_changed_at.unwrap_or(t.updated_at);
    let after_sync = |d: DateTime<Utc>| t.github_synced_at.is_none_or(|s| d > s + SLACK);
    let remote_changed = match base {
        Some(b) => issue.updated_at != b.updated_at,
        None => after_sync(issue.updated_at),
    };
    let prefer_remote = match base {
        Some(_) => issue.updated_at > changed,
        None => !after_sync(changed) || (remote_changed && issue.updated_at > changed),
    };
    let mut m = Merged::default();

    match pick(t.title.as_str(), issue.title.as_str(), base.map(|b| b.title.as_str()), prefer_remote) {
        Side::Same => {}
        Side::Local => m.push.title = Some(t.title.clone()),
        Side::Remote => m.title = Some(issue.title.clone()),
    }
    let (lb, rb) = (unix_lines(&t.body), unix_lines(&issue.body));
    match pick(&lb, &rb, base.map(|b| unix_lines(&b.body)).as_ref(), prefer_remote) {
        Side::Same => {}
        Side::Local => m.push.body = Some(t.body.clone()),
        Side::Remote => m.body = Some(issue.body.clone()),
    }

    let local = labels_from_tags(&t.tags);
    let labels: Vec<String> = match base {
        // fusion d'ensembles : présent des deux côtés, ou ajouté d'un côté depuis la base
        Some(b) => {
            let mut out: Vec<String> =
                issue.labels.iter().filter(|l| has_label(&local, l) || !has_label(&b.labels, l)).cloned().collect();
            out.extend(local.iter().filter(|l| !has_label(&issue.labels, l) && !has_label(&b.labels, l)).cloned());
            out
        }
        None if same_labels(&local, &issue.labels) || prefer_remote => issue.labels.clone(),
        None => local.clone(),
    };
    if !same_labels(&labels, &issue.labels) {
        m.push.labels = Some(labels.clone());
    }
    if !same_labels(&labels, &local) {
        // les tags déjà là gardent leur nom et leur ordre, les nouveaux labels suivent
        let mut tags: Vec<String> =
            t.tags.iter().filter(|tag| has_label(&labels, &tag_to_label(tag))).cloned().collect();
        tags.extend(tags_from_labels(&labels).into_iter().filter(|tag| !has_label(&local, &tag_to_label(tag))));
        m.tags = Some(tags);
    }

    let status = if issue.pull {
        if remote_changed { status_from_pr(t.status, issue) } else { t.status }
    } else {
        let open = t.status != Status::Done;
        match pick(&open, &issue.open, base.map(|b| &b.open), prefer_remote) {
            Side::Same => t.status,
            Side::Local => {
                m.push.open = Some(open);
                t.status
            }
            Side::Remote => status_from_issue(t.status, issue.open),
        }
    };
    if status != t.status {
        m.status = Some(status);
    }
    m
}

/// Decides what to do for each issue of the repository: import it, or merge it with its ticket (`merge_ticket`).
/// Les tickets déjà à jour (rien à recevoir ni à envoyer, base et état identiques à l'issue) sont passés.
pub fn plan_sync(tickets: &[&Ticket], issues: &[RemoteIssue]) -> Vec<SyncAction> {
    let mut out = vec![];
    for issue in issues {
        match tickets.iter().find(|t| t.github_number == Some(issue.number)) {
            None => out.push(SyncAction::Import(issue.clone())),
            Some(t) => {
                let merged = merge_ticket(t, issue);
                let fresh = t.github_base.as_ref() == Some(&IssueBase::of(issue))
                    && t.github_state.as_deref() == Some(remote_state(issue))
                    && t.github_pr == issue.pull;
                if !fresh || merged.pulls() || !merged.push.is_empty() {
                    out.push(SyncAction::Update { ticket: t.id, issue: issue.clone(), merged });
                }
            }
        }
    }
    out
}

/// Ticket status after pulling an issue: a closed issue is done; a re-opened one goes back to "to do";
/// otherwise the finer local status (in progress, blocked, review) is kept.
pub fn status_from_issue(current: Status, open: bool) -> Status {
    match (open, current) {
        (false, _) => Status::Done,
        (true, Status::Done) => Status::Todo,
        (true, s) => s,
    }
}

/// Statut d'un ticket d'après sa pull request : fusionnée / fermée → Terminé, brouillon → En cours,
/// ouverte → En revue (sauf si le ticket est déjà En cours ou Bloqué).
pub fn status_from_pr(current: Status, issue: &RemoteIssue) -> Status {
    if !issue.open {
        return Status::Done;
    }
    match (issue.draft, current) {
        (true, _) => Status::Doing,
        (false, Status::Doing | Status::Blocked) => current,
        (false, _) => Status::Review,
    }
}

/// Statut d'un ticket d'après son issue ou sa pull request.
pub fn status_from_remote(current: Status, issue: &RemoteIssue) -> Status {
    if issue.pull { status_from_pr(current, issue) } else { status_from_issue(current, issue.open) }
}

/// État GitHub affiché : open / closed, et merged / draft pour les pull requests.
pub fn remote_state(issue: &RemoteIssue) -> &'static str {
    match (issue.pull, issue.open, issue.merged, issue.draft) {
        (true, false, true, _) => "merged",
        (true, true, _, true) => "draft",
        (_, true, _, _) => "open",
        _ => "closed",
    }
}

/// Labels GitHub → tags du ticket (sans doublon) : le nom du tag intégré quand le label est le sien
/// (`enhancement` → « Amélioration »), sinon le nom du label sur le dépôt.
pub fn tags_from_labels(labels: &[String]) -> Vec<String> {
    let mut tags: Vec<String> = vec![];
    for t in labels.iter().map(|l| label_to_tag(l)) {
        if !tags.iter().any(|x| same_label(x, &t)) {
            tags.push(t);
        }
    }
    tags
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + s, 0).unwrap()
    }

    /// Ticket sans base (synchronisé par une ancienne version).
    fn ticket(n: Option<i32>, updated: i64, synced: Option<i64>) -> Ticket {
        Ticket {
            id: Uuid::new_v4(),
            project_id: Uuid::nil(),
            number: 1,
            title: "t".into(),
            body: String::new(),
            status: Status::Todo,
            tags: vec![],
            priority: 1,
            github_number: n,
            github_url: None,
            github_state: Some("open".into()),
            github_synced_at: synced.map(at),
            github_pr: false,
            github_base: None,
            created_at: at(0),
            updated_at: at(updated),
            content_changed_at: Some(at(updated)),
            done_at: None,
        }
    }
    fn issue(n: i32, updated: i64) -> RemoteIssue {
        RemoteIssue {
            pull: false,
            draft: false,
            merged: false,
            number: n,
            title: "i".into(),
            body: String::new(),
            open: true,
            labels: vec![],
            url: String::new(),
            updated_at: at(updated),
        }
    }
    /// Ticket et issue identiques, synchronisés à t = 100.
    fn synced() -> (Ticket, RemoteIssue) {
        let i =
            RemoteIssue { title: "Titre".into(), body: "Corps".into(), labels: vec!["bug".into()], ..issue(5, 100) };
        let t = Ticket {
            title: i.title.clone(),
            body: i.body.clone(),
            tags: vec!["Bug".into()],
            github_base: Some(IssueBase::of(&i)),
            ..ticket(Some(5), 100, Some(100))
        };
        (t, i)
    }
    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn sync_plan_without_base() {
        // tickets d'une ancienne version : les dates décident, comme avant
        let unchanged = ticket(Some(1), 100, Some(100));
        let local = ticket(Some(2), 500, Some(100));
        let remote = ticket(Some(3), 100, Some(100));
        let both = ticket(Some(4), 300, Some(100));
        let same = Ticket { title: "i".into(), ..ticket(Some(5), 100, Some(100)) };
        let ts = [&unchanged, &local, &remote, &both, &same];
        let plan =
            plan_sync(&ts, &[issue(1, 100), issue(2, 100), issue(3, 400), issue(4, 600), issue(5, 100), issue(9, 50)]);
        let title = |a: &SyncAction| match a {
            SyncAction::Update { issue, merged, .. } => (issue.number, merged.title.clone(), merged.push.title.clone()),
            SyncAction::Import(i) => (-i.number, None, None),
        };
        let got: Vec<_> = plan.iter().map(title).collect();
        assert_eq!(
            got,
            vec![
                (1, Some("i".into()), None), // aucun côté n'a bougé mais ils diffèrent : GitHub fait foi
                (2, None, Some("t".into())), // modifié localement : envoyé
                (3, Some("i".into()), None), // modifié sur GitHub : reçu
                (4, Some("i".into()), None), // les deux : le plus récent (GitHub)
                (5, None, None),             // identiques : seule la base est enregistrée
                (-9, None, None),
            ]
        );
        let both_local = ticket(Some(4), 700, Some(100));
        assert_eq!(merge_ticket(&both_local, &issue(4, 600)).push.title.as_deref(), Some("t"));
    }

    #[test]
    fn up_to_date_tickets_are_skipped() {
        let (t, i) = synced();
        assert!(plan_sync(&[&t], std::slice::from_ref(&i)).is_empty());
        // priorité ou projet modifiés (updated_at bouge, pas content_changed_at) : rien à envoyer
        let touched = Ticket { updated_at: at(900), priority: 3, ..t.clone() };
        assert!(plan_sync(&[&touched], std::slice::from_ref(&i)).is_empty());
        // statut « en cours » : toujours ouvert pour GitHub
        let doing = Ticket { status: Status::Doing, updated_at: at(900), ..t.clone() };
        assert!(plan_sync(&[&doing], std::slice::from_ref(&i)).is_empty());
        // commentaire sur l'issue (updated_at seul) : rien à fusionner, mais la base est rafraîchie
        let commented = RemoteIssue { updated_at: at(800), ..i.clone() };
        let plan = plan_sync(&[&t], &[commented]);
        assert!(matches!(&plan[..], [SyncAction::Update { merged, .. }] if *merged == Merged::default()));
    }

    #[test]
    fn merge_per_field() {
        let (t, i) = synced();
        // faute corrigée dans le titre du ticket, issue fermée entre-temps sur GitHub : le titre part,
        // la fermeture arrive — l'issue n'est pas rouverte
        let local = Ticket { title: "Titre corrigé".into(), content_changed_at: Some(at(900)), ..t.clone() };
        let closed = RemoteIssue { open: false, updated_at: at(500), ..i.clone() };
        let m = merge_ticket(&local, &closed);
        assert_eq!(m.push, IssuePatch { title: Some("Titre corrigé".into()), ..Default::default() });
        assert_eq!((m.status, m.title.clone(), m.body.clone(), m.tags.clone()), (Some(Status::Done), None, None, None));

        // description modifiée sur GitHub, ticket terminé localement : chacun reçoit le champ de l'autre
        let done = Ticket { status: Status::Done, content_changed_at: Some(at(900)), ..t.clone() };
        let edited = RemoteIssue { body: "Corps\r\nsuite".into(), updated_at: at(950), ..i.clone() };
        let m = merge_ticket(&done, &edited);
        assert_eq!(m.push, IssuePatch { open: Some(false), ..Default::default() });
        assert_eq!((m.body.as_deref(), m.status), (Some("Corps\r\nsuite"), None));

        // même champ modifié des deux côtés : le plus récent gagne
        let remote_title = RemoteIssue { title: "Titre GitHub".into(), updated_at: at(950), ..i.clone() };
        let m = merge_ticket(&local, &remote_title);
        assert_eq!((m.title.as_deref(), m.push.is_empty()), (Some("Titre GitHub"), true));
        let m = merge_ticket(&Ticket { content_changed_at: Some(at(990)), ..local.clone() }, &remote_title);
        assert_eq!((m.title, m.push.title.as_deref()), (None, Some("Titre corrigé")));

        // rouverte sur GitHub : le ticket terminé revient « à faire » ; « en cours » reste tel quel
        let (mut t2, mut i2) = synced();
        t2.status = Status::Done;
        i2.open = false;
        t2.github_base = Some(IssueBase::of(&i2));
        let reopened = RemoteIssue { open: true, updated_at: at(700), ..i2.clone() };
        assert_eq!(merge_ticket(&t2, &reopened).status, Some(Status::Todo));
        // fins de ligne : pas une différence
        let crlf = RemoteIssue { body: "a\r\nb".into(), ..i.clone() };
        let t3 = Ticket { body: "a\nb".into(), github_base: Some(IssueBase::of(&crlf)), ..t.clone() };
        assert_eq!(merge_ticket(&t3, &crlf), Merged::default());
    }

    #[test]
    fn merge_labels() {
        let (mut t, mut i) = synced();
        i.labels = strs(&["bug", "enhancement", "help wanted"]);
        t.tags = strs(&["Bug", "Amélioration", "help wanted"]);
        t.github_base = Some(IssueBase::of(&i));
        // les tags intégrés font l'aller-retour sans changer de nom
        assert_eq!(merge_ticket(&t, &i), Merged::default());
        assert!(plan_sync(&[&t], std::slice::from_ref(&i)).is_empty());
        // ajout local « Tâche technique » + retrait de « Bug » ; ajout de « security » et retrait de
        // « help wanted » sur GitHub : tout est repris
        let local = Ticket { tags: strs(&["Amélioration", "help wanted", "Tâche technique"]), ..t.clone() };
        let remote =
            RemoteIssue { labels: strs(&["bug", "enhancement", "security"]), updated_at: at(300), ..i.clone() };
        let m = merge_ticket(&local, &remote);
        assert_eq!(m.push.labels, Some(strs(&["enhancement", "security", "chore"])));
        assert_eq!(m.tags, Some(strs(&["Amélioration", "Tâche technique", "Sécurité"])));
        assert_eq!((m.title, m.status, m.push.title, m.push.open), (None, None, None, None));
        // la casse d'un label n'est pas une différence
        let cased = RemoteIssue { labels: strs(&["Bug", "Enhancement", "Help Wanted"]), ..i.clone() };
        assert_eq!(merge_ticket(&t, &cased), Merged::default());
    }

    #[test]
    fn labels_and_status() {
        assert_eq!(tag_to_label("Bug"), "bug");
        assert_eq!(tag_to_label("Mon tag"), "Mon tag");
        assert_eq!(tag_to_label("good first issue"), "good first issue");
        assert_eq!(status_from_issue(Status::Doing, true), Status::Doing);
        assert_eq!(status_from_issue(Status::Doing, false), Status::Done);
        assert_eq!(status_from_issue(Status::Done, true), Status::Todo);
        // chaque tag intégré fait l'aller-retour tag → label → tag
        for t in TAGS {
            assert_eq!(label_to_tag(&tag_to_label(t.0)), t.0);
            assert_eq!(tag_to_label(&label_to_tag(t.2)), t.2);
        }
        assert_eq!(label_to_tag("Enhancement"), "Amélioration");
        assert_eq!(label_to_tag("chore"), "Tâche technique");
        assert_eq!(label_to_tag("good first issue"), "good first issue");
        // les autres labels gardent le nom du dépôt ; pas de doublon
        assert_eq!(
            tags_from_labels(&strs(&["bug", "Bug", "good first issue", "enhancement"])),
            vec!["Bug", "good first issue", "Amélioration"]
        );
        assert_eq!(labels_from_tags(&strs(&["Bug", "bug", "Amélioration", "x"])), vec!["bug", "enhancement", "x"]);
    }

    #[test]
    fn base_json_round_trip() {
        let (_, i) = synced();
        let b = IssueBase::of(&i);
        assert_eq!(IssueBase::from_json(&b.to_json()), Some(b));
        assert_eq!(IssueBase::from_json(&serde_json::json!({"title": "x"})), None);
    }

    #[test]
    fn pull_requests() {
        let pr = |open, draft, merged| RemoteIssue { pull: true, open, draft, merged, ..issue(1, 0) };
        assert_eq!(status_from_remote(Status::Todo, &pr(true, false, false)), Status::Review);
        assert_eq!(status_from_remote(Status::Doing, &pr(true, false, false)), Status::Doing);
        assert_eq!(status_from_remote(Status::Todo, &pr(true, true, false)), Status::Doing);
        assert_eq!(status_from_remote(Status::Review, &pr(false, false, true)), Status::Done);
        assert_eq!(remote_state(&pr(false, false, true)), "merged");
        assert_eq!(remote_state(&pr(false, false, false)), "closed");
        assert_eq!(remote_state(&pr(true, true, false)), "draft");
        assert_eq!(remote_state(&issue(1, 0)), "open");
        // une PR n'est jamais fermée ni rouverte depuis Mnemo ; son état ne donne le statut que s'il a bougé
        let (mut t, mut i) = synced();
        i.pull = true;
        t.github_pr = true;
        t.status = Status::Review;
        t.github_base = Some(IssueBase::of(&i));
        let done = Ticket { status: Status::Done, title: "Nouveau".into(), ..t.clone() };
        let m = merge_ticket(&done, &i);
        assert_eq!((m.push.open, m.push.title.as_deref(), m.status), (None, Some("Nouveau"), None));
        let merged_pr = RemoteIssue { open: false, merged: true, updated_at: at(400), ..i.clone() };
        assert_eq!(merge_ticket(&t, &merged_pr).status, Some(Status::Done));
    }
}
