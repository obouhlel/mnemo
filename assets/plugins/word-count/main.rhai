// Hook appelé à chaque modification de la note ouverte.
fn status(note) {
    if type_of(note) == "()" { return (); }
    let words = word_count(note.content);
    let chars = char_count(note.content);
    let wpm = setting("wpm");
    if type_of(wpm) != "i64" || wpm <= 0 { wpm = 200; }
    let minutes = (words + wpm - 1) / wpm;
    `${words} mots · ${chars} caractères · ~${minutes} min`
}
