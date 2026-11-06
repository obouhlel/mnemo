// Transforme le markdown avant l'affichage de l'aperçu (la note n'est pas modifiée).
fn preprocess(md, note) {
    let icons = #{
        note: "ℹ", info: "ℹ", tip: "💡", success: "✔", question: "❓",
        warning: "⚠", danger: "⛔", bug: "🐞", todo: "☑", quote: "❝", example: "✏"
    };
    let res = "";
    let first = true;
    for line in md.split("\n") {
        if !first { res += "\n"; }
        first = false;
        if line.starts_with("> [!") {
            let end = line.index_of("]");
            if end > 4 {
                let kind = line.sub_string(4, end - 4);
                kind.make_lower();
                let title = line.sub_string(end + 1);
                title.trim();
                if title == "" { title = kind.to_upper(); }
                let icon = icons[kind] ?? "ℹ";
                res += `> **${icon}  ${title}**` + "\n>";
                continue;
            }
        }
        res += line;
    }
    res
}
