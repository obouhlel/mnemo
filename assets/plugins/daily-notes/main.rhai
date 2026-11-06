fn commands() {
    [
        #{ id: "today",     name: "Ouvrir la note du jour" },
        #{ id: "yesterday", name: "Ouvrir la note d'hier" },
        #{ id: "tomorrow",  name: "Ouvrir la note de demain" },
    ]
}

fn open_day(offset) {
    let folder = setting("folder");
    let name = date_offset(offset, setting("format"));
    let path = if folder == "" { name } else { `${folder}/${name}` };
    let content = setting("template");
    content.replace("{{date}}", name);
    // create_note ouvre la note si elle existe déjà
    create_note(path, content);
}

fn run(id, note) {
    switch id {
        "today" => open_day(0),
        "yesterday" => open_day(-1),
        "tomorrow" => open_day(1),
        _ => ()
    }
}
