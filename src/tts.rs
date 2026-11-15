//! Lecture à voix haute avec la synthèse vocale du système, sans dépendance à compiler : speech-dispatcher
//! (`spd-say`) ou espeak-ng sous Linux, `say` sous macOS, la voix de Windows (System.Speech) via PowerShell.
//! Le texte part sur l'entrée standard du programme, qui lit en arrière-plan pendant qu'on travaille.

use std::io::Write;
use std::process::{Child, Command, Stdio};

/// Texte d'une note tel qu'il doit être lu : sans frontmatter, code, balisage Markdown ni adresses ;
/// `[[cible|alias]]` et `[texte](adresse)` sont lus par leur texte, les images et intégrations sautées.
pub fn speakable(markdown: &str) -> String {
    let body = match crate::markdown::frontmatter(markdown) {
        Some((_, at)) => &markdown[at..],
        None => markdown,
    };
    let mut out = String::with_capacity(body.len());
    let mut in_code = false;
    for line in body.lines() {
        let t = line.trim();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_code = !in_code;
            continue;
        }
        if in_code || t.starts_with("$$") || t.chars().all(|c| matches!(c, '-' | '*' | '_' | '=' | '|' | ':' | ' ')) {
            // code, formules, règles horizontales et lignes de séparation des tableaux : pas lus
            if !out.ends_with("\n\n") && !out.is_empty() {
                out.push('\n');
            }
            continue;
        }
        let t = t.trim_start_matches('#').trim_start_matches('>').trim_start();
        // puces, numéros et cases de tâches
        let t = t.strip_prefix("- [ ] ").or_else(|| t.strip_prefix("- [x] ")).unwrap_or(t);
        let t = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")).or_else(|| t.strip_prefix("+ ")).unwrap_or(t);
        let t = match t.split_once(". ") {
            Some((n, rest)) if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) => rest,
            _ => t,
        };
        // callout `[!note] Titre`
        let t = match t.strip_prefix("[!").and_then(|r| r.split_once(']')) {
            Some((_, title)) => title.trim_start_matches(['-', '+']).trim(),
            None => t,
        };
        let text = inline_text(t).replace('|', ", ");
        let text = text.trim();
        if text.is_empty() {
            if !out.is_empty() && !out.ends_with("\n\n") {
                out.push('\n');
            }
            continue;
        }
        out.push_str(text);
        // fin de phrase marquée pour que la voix fasse une pause entre les lignes
        if !text.ends_with(['.', '!', '?', ':', ';', ',']) {
            out.push('.');
        }
        out.push('\n');
    }
    out.trim().to_string()
}

/// Texte d'une ligne sans balisage en ligne.
fn inline_text(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(c) = rest.chars().next() {
        // ![[intégration]] et ![image](…) : sautés
        if rest.starts_with("![[") {
            rest = rest.find("]]").map_or("", |e| &rest[e + 2..]);
            continue;
        }
        if rest.starts_with("![") {
            rest = rest.find(')').map_or("", |e| &rest[e + 1..]);
            continue;
        }
        if let Some(inner) = rest.strip_prefix("[[")
            && let Some(end) = inner.find("]]")
        {
            out.push_str(&crate::markdown::parse_wikilink(&inner[..end]).1);
            rest = &inner[end + 2..];
            continue;
        }
        if let Some(inner) = rest.strip_prefix('[')
            && let Some(close) = inner.find("](")
            && let Some(end) = inner[close..].find(')')
        {
            out.push_str(&inner[..close]);
            rest = &inner[close + end + 1..];
            continue;
        }
        if rest.starts_with("http://") || rest.starts_with("https://") {
            rest = rest.find(char::is_whitespace).map_or("", |e| &rest[e..]);
            continue;
        }
        let word_start = out.chars().last().is_none_or(char::is_whitespace);
        if matches!(c, '*' | '_' | '`' | '=' | '~' | '$') || (c == '#' && word_start) {
            rest = &rest[c.len_utf8()..];
            continue;
        }
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// Lecture en cours (programme de synthèse vocale lancé).
#[derive(Default)]
pub struct Speaker {
    child: Option<Child>,
    /// programme utilisé (pour l'arrêter proprement)
    program: Option<&'static str>,
}

impl Speaker {
    /// Vrai pendant la lecture.
    pub fn speaking(&mut self) -> bool {
        match &mut self.child {
            Some(c) => match c.try_wait() {
                Ok(None) => true,
                _ => {
                    self.child = None;
                    false
                }
            },
            None => false,
        }
    }

    /// Lit `text` (arrête la lecture précédente). `rate` : vitesse, 1.0 = normale (0.5 à 2.0).
    pub fn speak(&mut self, text: &str, rate: f32) -> Result<(), String> {
        self.stop();
        if text.trim().is_empty() {
            return Err("Rien à lire dans cette note".into());
        }
        let rate = rate.clamp(0.5, 2.0);
        let mut last_err = String::new();
        for (program, args) in commands(rate) {
            let mut cmd = Command::new(program);
            cmd.args(&args).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null());
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW : pas de console qui s'ouvre
            }
            match cmd.spawn() {
                Ok(mut child) => {
                    if let Some(mut stdin) = child.stdin.take() {
                        // écrit dans un fil à part : un long texte remplirait le tube et bloquerait l'interface
                        let text = text.to_string();
                        std::thread::spawn(move || {
                            let _ = stdin.write_all(text.as_bytes());
                        });
                    }
                    self.child = Some(child);
                    self.program = Some(program);
                    return Ok(());
                }
                Err(e) => last_err = format!("{program} : {e}"),
            }
        }
        Err(format!("Aucune synthèse vocale trouvée sur ce système ({}). {INSTALL_HINT}", last_err))
    }

    /// Arrête la lecture.
    pub fn stop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        // speech-dispatcher continue de lire ce qu'il a reçu : on lui demande d'arrêter
        if self.program.take() == Some("spd-say") {
            let _ = Command::new("spd-say").arg("--cancel").stdout(Stdio::null()).stderr(Stdio::null()).status();
        }
    }
}

impl Drop for Speaker {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(target_os = "linux")]
const INSTALL_HINT: &str = "Installez speech-dispatcher (spd-say) ou espeak-ng.";
#[cfg(not(target_os = "linux"))]
const INSTALL_HINT: &str = "";

/// Programmes essayés dans l'ordre, avec leurs arguments (texte lu sur l'entrée standard).
fn commands(rate: f32) -> Vec<(&'static str, Vec<String>)> {
    #[cfg(target_os = "macos")]
    {
        // mots par minute (175 par défaut)
        vec![("say", vec!["-r".into(), format!("{}", (175.0 * rate) as i32), "-f".into(), "-".into()])]
    }
    #[cfg(windows)]
    {
        // vitesse de -10 à 10 (0 par défaut)
        let r = ((rate - 1.0) * 10.0).round().clamp(-10.0, 10.0) as i32;
        let script = format!(
            "[Console]::InputEncoding=[Text.Encoding]::UTF8; Add-Type -AssemblyName System.Speech; \
             $s=New-Object System.Speech.Synthesis.SpeechSynthesizer; $s.Rate={r}; \
             $s.Speak([Console]::In.ReadToEnd())"
        );
        vec![("powershell", vec!["-NoProfile".into(), "-NonInteractive".into(), "-Command".into(), script])]
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        // spd-say : vitesse de -100 à 100 ; espeak-ng : mots par minute (175 par défaut)
        let spd = ((rate - 1.0) * 100.0).round().clamp(-100.0, 100.0) as i32;
        let wpm = format!("{}", (175.0 * rate) as i32);
        vec![
            ("spd-say", vec!["-l".into(), "fr".into(), "-r".into(), spd.to_string(), "-e".into(), "-w".into()]),
            ("espeak-ng", vec!["-v".into(), "fr".into(), "-s".into(), wpm.clone(), "--stdin".into()]),
            ("espeak", vec!["-v".into(), "fr".into(), "-s".into(), wpm, "--stdin".into()]),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::speakable;

    #[test]
    fn text_to_read() {
        let md = "---\ntags: [a]\n---\n# Titre\n\nVoir [[Note cible|la cible]] et [site](https://x.fr) **gras** `code`\n\
                  ```rust\nfn main() {}\n```\n- [x] Fait\n1. Premier\n![[image.png]]\n> [!note] Attention\n---\nFin #tag";
        assert_eq!(
            speakable(md),
            "Titre.\n\nVoir la cible et site gras code.\n\nFait.\nPremier.\n\nAttention.\n\nFin tag."
        );
    }
}
