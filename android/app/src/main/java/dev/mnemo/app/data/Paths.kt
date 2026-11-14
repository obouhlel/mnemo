package dev.mnemo.app.data

private fun fail(message: String): Result<Nothing> = Result.failure(IllegalArgumentException(message))

/**
 * Normalise un chemin de note saisi par l'utilisateur, exactement comme Mnemo sur ordinateur (`normalize_path`,
 * src/model.rs) : espaces et `.md` retirés, segments vides ignorés, 500 caractères au plus et 250 octets par nom,
 * pas de `[ ] | # ^ \` ni de caractère de contrôle, pas de segment commençant par un point (donc ni `.` ni `..`).
 * Échec : message en français.
 */
fun normalizePath(raw: String): Result<String> {
    val trimmed = raw.trim().let { t -> if (t.endsWith(".md") || t.endsWith(".MD")) t.dropLast(3) else t }
    val segments = trimmed.split('/').map { it.trim() }.filter { it.isNotEmpty() }
    val p = segments.joinToString("/")
    return when {
        p.isEmpty() -> fail("Le nom ne peut pas être vide")
        p.codePointCount(0, p.length) > 500 -> fail("Nom trop long")
        p.any { it in "[]|#^\\" || it.isISOControl() } -> fail("Caractères interdits : [ ] | # ^ \\")
        segments.any { it.startsWith('.') } -> fail("Un nom ne peut pas commencer par un point")
        segments.any { it.toByteArray(Charsets.UTF_8).size > 250 } -> fail("Nom trop long")
        else -> Result.success(p)
    }
}

private val RESERVED = setOf("CON", "PRN", "AUX", "NUL")

/**
 * Vérifie qu'un nouveau nom existe sur tous les systèmes (`portable_name`, src/model.rs) : pas de `: * ? " < >`,
 * pas de point ou d'espace final, pas de nom réservé par Windows. Renvoie le message d'erreur, ou null.
 */
fun portableName(path: String): String? {
    for (seg in path.split('/')) {
        if (seg.any { it in ":*?\"<>" }) return "Caractères interdits dans un nom : : * ? \" < >"
        if (seg.endsWith('.') || seg.endsWith(' ')) return "Un nom ne peut pas finir par un point ou un espace"
        val stem = seg.substringBefore('.').uppercase(java.util.Locale.ROOT)
        val reserved = stem in RESERVED ||
            (stem.length == 4 && (stem.startsWith("COM") || stem.startsWith("LPT")) && stem[3] in '1'..'9')
        if (reserved) return "« $seg » est un nom réservé par Windows"
    }
    return null
}

/** Chemin d'une note créée ou renommée sur l'appareil : normalisé, et valable sur tous les systèmes. */
fun newNotePath(raw: String): Result<String> =
    normalizePath(raw).fold({ p -> portableName(p)?.let { fail(it) } ?: Result.success(p) }, { Result.failure(it) })

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
 * l'appareil, et les notes déjà nommées ainsi sur ordinateur restent lisibles).
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
