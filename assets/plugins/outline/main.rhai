// Retourne du markdown affiché dans le panneau latéral droit.
fn panel(note) {
    if type_of(note) == "()" { return (); }
    let out = "";
    let in_code = false;
    for line in note.content.split("\n") {
        let t = line;
        t.trim();
        if t.starts_with("```") { in_code = !in_code; continue; }
        if in_code || !t.starts_with("#") { continue; }
        let level = 0;
        while level < t.len() && t.sub_string(level, 1) == "#" { level += 1; }
        if level > 6 || t.sub_string(level, 1) != " " { continue; }
        let title = t.sub_string(level + 1);
        title.trim();
        let indent = "";
        for i in 1..level { indent += "  "; }
        out += `${indent}- ${title}` + "\n";
    }
    if out == "" { "*Aucun titre*" } else { out }
}
