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

pub fn tag_color(tag: &str) -> Color32 {
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

/// GitHub label -> ticket tag
pub fn label_to_tag(label: &str) -> String {
    let l = label.to_lowercase();
    let l = l.trim_start_matches("type:").trim_start_matches("kind/").trim();
    match l {
        "bug" | "defect" => "Bug".into(),
        "feature" | "feature request" | "new feature" => "Feature".into(),
        "enhancement" | "improvement" => "Amélioration".into(),
        "documentation" | "docs" => "Documentation".into(),
        "chore" | "maintenance" | "refactor" | "tech debt" => "Tâche technique".into(),
        "security" => "Sécurité".into(),
        "performance" | "perf" => "Performance".into(),
        "research" | "spike" | "investigation" => "Recherche".into(),
        _ => TAGS.iter().find(|t| t.0.eq_ignore_ascii_case(label)).map(|t| t.0.to_string()).unwrap_or_else(|| label.to_string()),
    }
}

#[derive(Debug, Clone)]
pub struct Project {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub github_repo: Option<String>,
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
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub done_at: Option<DateTime<Utc>>,
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
    pub number: i32,
    pub title: String,
    pub body: String,
    pub open: bool,
    pub labels: Vec<String>,
    pub url: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub enum SyncAction {
    /// Issue unknown locally: create a ticket
    Import(RemoteIssue),
    /// Issue changed on GitHub since the last sync: update the ticket
    Pull { ticket: Uuid, issue: RemoteIssue },
    /// Ticket changed locally since the last sync: update the issue
    Push {
        ticket: Uuid,
        #[allow(dead_code)]
        number: i32,
    },
}

/// Margin to ignore the few ms between `updated_at` and `github_synced_at` set by the same statement
const SLACK: chrono::Duration = chrono::Duration::seconds(2);

/// Decides what to do for each issue of the repository. Rule: the side that changed since the last
/// sync wins; if both changed, the most recent change wins.
pub fn plan_sync(tickets: &[&Ticket], issues: &[RemoteIssue]) -> Vec<SyncAction> {
    let mut out = vec![];
    for issue in issues {
        match tickets.iter().find(|t| t.github_number == Some(issue.number)) {
            None => out.push(SyncAction::Import(issue.clone())),
            Some(t) => {
                let local = t.github_synced_at.is_none_or(|s| t.updated_at > s + SLACK);
                let remote = t.github_synced_at.is_none_or(|s| issue.updated_at > s + SLACK);
                let pull = match (local, remote) {
                    (false, false) => continue,
                    (true, false) => false,
                    (false, true) => true,
                    (true, true) => issue.updated_at > t.updated_at,
                };
                out.push(if pull {
                    SyncAction::Pull { ticket: t.id, issue: issue.clone() }
                } else {
                    SyncAction::Push { ticket: t.id, number: issue.number }
                });
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

pub fn tags_from_labels(labels: &[String]) -> Vec<String> {
    let mut tags: Vec<String> = vec![];
    for l in labels {
        let t = label_to_tag(l);
        if !tags.contains(&t) {
            tags.push(t);
        }
    }
    tags
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticket(n: Option<i32>, updated: i64, synced: Option<i64>) -> Ticket {
        let at = |s: i64| DateTime::from_timestamp(1_700_000_000 + s, 0).unwrap();
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
            github_state: None,
            github_synced_at: synced.map(at),
            created_at: at(0),
            updated_at: at(updated),
            done_at: None,
        }
    }
    fn issue(n: i32, updated: i64) -> RemoteIssue {
        RemoteIssue {
            number: n,
            title: "i".into(),
            body: String::new(),
            open: true,
            labels: vec![],
            url: String::new(),
            updated_at: DateTime::from_timestamp(1_700_000_000 + updated, 0).unwrap(),
        }
    }

    #[test]
    fn sync_plan() {
        let unchanged = ticket(Some(1), 100, Some(100));
        let local = ticket(Some(2), 500, Some(100));
        let remote = ticket(Some(3), 100, Some(100));
        let both = ticket(Some(4), 300, Some(100));
        let ts = [&unchanged, &local, &remote, &both];
        let plan = plan_sync(&ts, &[issue(1, 100), issue(2, 100), issue(3, 400), issue(4, 600), issue(9, 50)]);
        assert_eq!(plan.len(), 4);
        assert!(matches!(&plan[0], SyncAction::Push { number: 2, .. }));
        assert!(matches!(&plan[1], SyncAction::Pull { issue, .. } if issue.number == 3));
        assert!(matches!(&plan[2], SyncAction::Pull { issue, .. } if issue.number == 4)); // remote more recent
        assert!(matches!(&plan[3], SyncAction::Import(i) if i.number == 9));
    }

    #[test]
    fn labels_and_status() {
        assert_eq!(label_to_tag("bug"), "Bug");
        assert_eq!(label_to_tag("type: feature"), "Feature");
        assert_eq!(label_to_tag("enhancement"), "Amélioration");
        assert_eq!(label_to_tag("wontfix"), "wontfix");
        assert_eq!(tag_to_label("Bug"), "bug");
        assert_eq!(tag_to_label("Mon tag"), "Mon tag");
        for (tag, _, label, _) in TAGS {
            assert_eq!(label_to_tag(label), *tag, "round-trip {tag}");
        }
        assert_eq!(status_from_issue(Status::Doing, true), Status::Doing);
        assert_eq!(status_from_issue(Status::Doing, false), Status::Done);
        assert_eq!(status_from_issue(Status::Done, true), Status::Todo);
        assert_eq!(tags_from_labels(&["bug".into(), "defect".into(), "x".into()]), vec!["Bug", "x"]);
    }
}
