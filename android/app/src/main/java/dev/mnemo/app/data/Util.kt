package dev.mnemo.app.data

/**
 * Empreinte d'une pièce jointe : FNV-1a 64 bits en hexadécimal (16 caractères),
 * exactement comme `file_hash` de Mnemo sur ordinateur (src/db/vault_sync.rs).
 */
fun fileHash(bytes: ByteArray): String {
    var h = -0x340d631b7bdddcdbL // 0xcbf29ce484222325
    for (b in bytes) {
        h = h xor (b.toLong() and 0xff)
        h *= 0x100000001b3L
    }
    return java.lang.Long.toHexString(h).padStart(16, '0')
}

/** Nom de fichier d'un chemin « dossier/sous/nom ». */
fun basename(path: String): String = path.substringAfterLast('/')

/** Dossier parent d'un chemin, ou null à la racine. */
fun parentOf(path: String): String? = path.lastIndexOf('/').takeIf { it > 0 }?.let { path.substring(0, it) }

/** Paramètres de connexion JDBC tirés d'une adresse `postgres://utilisateur:motdepasse@hôte:5432/base?sslmode=require`. */
data class DbConfig(val jdbcUrl: String, val user: String, val password: String, val ssl: Boolean, val display: String)

fun parseDbUrl(raw: String): DbConfig? {
    val url = raw.trim()
    val rest = url.removePrefix("postgresql://").removePrefix("postgres://").takeIf { it != url } ?: return null
    val at = rest.lastIndexOf('@')
    val (creds, hostPart) = if (at >= 0) rest.substring(0, at) to rest.substring(at + 1) else "" to rest
    val user = java.net.URLDecoder.decode(creds.substringBefore(':'), "UTF-8")
    val password = if (creds.contains(':')) java.net.URLDecoder.decode(creds.substringAfter(':'), "UTF-8") else ""
    val query = hostPart.substringAfter('?', "")
    val hostDb = hostPart.substringBefore('?')
    val host = hostDb.substringBefore('/')
    val db = hostDb.substringAfter('/', "postgres").ifEmpty { "postgres" }
    if (host.isEmpty()) return null
    val params = query.split('&').filter { it.contains('=') }.associate { it.substringBefore('=') to it.substringAfter('=') }
    val ssl = params["sslmode"]?.let { it == "require" || it.startsWith("verify") } ?: false
    // sslmode=require : chiffré sans vérifier le certificat, comme libpq ; pas de lecture de ~/.postgresql
    val sslParams = if (ssl) "?ssl=true&sslfactory=org.postgresql.ssl.NonValidatingFactory" else "?sslmode=disable"
    val jdbc = "jdbc:postgresql://$host/$db$sslParams&ApplicationName=Mnemo%20Android&connectTimeout=10&socketTimeout=30&tcpKeepAlive=true"
    val display = "postgres://${if (user.isNotEmpty()) "$user@" else ""}$host/$db"
    return DbConfig(jdbc, user, password, ssl, display)
}
