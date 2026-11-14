package dev.mnemo.app.data

/** Caractères refusés dans un nom : syntaxe des liens (`[ ] | # ^ \`) et noms invalides sous Windows. */
private const val FORBIDDEN = "[]|#^\\:*?\"<>"

/**
 * Normalise un chemin de note saisi par l'utilisateur, avec les règles de Mnemo sur ordinateur
 * (`normalize_path`, src/model.rs) : espaces et `.md` retirés, segments vides ignorés, 500 caractères au plus,
 * caractères interdits, pas de segment `.`/`..`, ni commençant ou finissant par un point.
 * Échec : message en français.
 */
fun normalizePath(raw: String): Result<String> {
    val trimmed = raw.trim().let { t -> if (t.endsWith(".md") || t.endsWith(".MD")) t.dropLast(3) else t }
    val segments = trimmed.split('/').map { it.trim() }.filter { it.isNotEmpty() }
    val p = segments.joinToString("/")
    val error = when {
        p.isEmpty() -> "Le nom ne peut pas être vide"
        p.codePointCount(0, p.length) > 500 -> "Nom trop long"
        p.any { it in FORBIDDEN || it.isISOControl() } -> "Caractères interdits : [ ] | # ^ \\ : * ? \" < >"
        segments.any { it.startsWith('.') } -> "Un nom ne peut pas commencer par un point"
        segments.any { it.endsWith('.') } -> "Un nom ne peut pas finir par un point"
        else -> return Result.success(p)
    }
    return Result.failure(IllegalArgumentException(error))
}

/**
 * Chemin relatif sûr pour un fichier du coffre : il vient de la base, donc d'une autre machine, et ne doit jamais
 * désigner un fichier hors du dossier de l'appli (`..`, chemin absolu, lecteur Windows) ni un fichier caché.
 */
fun isSafeRelPath(path: String): Boolean {
    if (path.isEmpty() || path.length > 1000) return false
    if (path.any { it == '\\' || it == ':' || it.isISOControl() }) return false
    return path.split('/').none { it.isBlank() || it.startsWith('.') }
}

/**
 * Chemin de note acceptable venant de la base : mêmes règles, sauf `:` (une note n'est jamais un fichier sur
 * l'appareil, et d'anciennes versions du bureau acceptaient ce caractère dans un nom).
 */
fun isSafeNotePath(path: String): Boolean = isSafeRelPath(path.replace(':', '_'))

/** Clé de comparaison d'un chemin : sans casse, Unicode compris (« École » = « école », ce que `lower()` de SQLite ignore). */
fun pathKey(path: String): String = path.lowercase()

/** Premier nom libre : `base`, puis `base 2`, `base 3`… ([taken] reçoit le candidat). */
fun firstFree(base: String, ext: String = "", taken: (String) -> Boolean): String {
    var candidate = base + ext
    var n = 2
    while (taken(candidate)) candidate = "$base ${n++}$ext"
    return candidate
}

/** Nom de la copie gardée lors d'un conflit : à la seconde près (deux conflits dans la même minute ne s'écrasent pas). */
fun conflictName(path: String, now: java.util.Date = java.util.Date()): String =
    "$path (conflit mobile ${java.text.SimpleDateFormat("yyyy-MM-dd HHmmss", java.util.Locale.FRANCE).format(now)})"

/** Copie de conflit d'une pièce jointe (nom sans extension, extension) : le suffixe se place avant l'extension. */
fun fileConflictBase(path: String): Pair<String, String> {
    val dot = path.lastIndexOf('.')
    return if (dot > path.lastIndexOf('/') + 1) "${path.substring(0, dot)} (conflit mobile)" to path.substring(dot)
    else "$path (conflit mobile)" to ""
}
