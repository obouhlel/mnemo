fn commands() {
    [
        #{ id: "date",    name: "Insérer la date du jour" },
        #{ id: "time",    name: "Insérer l'heure" },
        #{ id: "todo",    name: "Insérer une case à cocher" },
        #{ id: "meeting", name: "Insérer un modèle de réunion" },
    ]
}

fn run(id, note) {
    if type_of(note) == "()" { notice("Ouvrez d'abord une note"); return; }
    switch id {
        "date" => insert_text(date(setting("date_format"))),
        "time" => insert_text(date("%H:%M")),
        "todo" => insert_text("- [ ] "),
        "meeting" => {
            let t = setting("meeting");
            t.replace("{{date}}", date(setting("date_format")));
            insert_text(t);
        }
        _ => ()
    }
}
