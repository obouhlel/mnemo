package dev.mnemo.app.data

/**
 * Empreinte d'une pièce jointe : FNV-1a 64 bits en hexadécimal (16 caractères),
 * exactement comme `file_hash` de Mnemo sur ordinateur (src/db/vault_sync.rs).
 */
fun fileHash(bytes: ByteArray): String = Fnv().apply { update(bytes) }.hex()

/** La même empreinte, calculée par morceaux (fichier lu ou reçu sans être chargé en entier en mémoire). */
class Fnv {
    private var h = -0x340d631b7bdddcdbL // 0xcbf29ce484222325

    fun update(buf: ByteArray, len: Int = buf.size) {
        for (i in 0 until len) {
            h = h xor (buf[i].toLong() and 0xff)
            h *= 0x100000001b3L
        }
    }

    fun hex(): String = java.lang.Long.toHexString(h).padStart(16, '0')
}

fun fileHash(input: java.io.InputStream): String {
    val fnv = Fnv()
    val buf = ByteArray(64 * 1024)
    while (true) {
        val n = input.read(buf)
        if (n < 0) break
        fnv.update(buf, n)
    }
    return fnv.hex()
}

/** Nom de fichier d'un chemin « dossier/sous/nom ». */
fun basename(path: String): String = path.substringAfterLast('/')

/** Dossier parent d'un chemin, ou null à la racine. */
fun parentOf(path: String): String? = path.lastIndexOf('/').takeIf { it > 0 }?.let { path.substring(0, it) }

private fun hexDigit(c: Char): Int = when (c) {
    in '0'..'9' -> c - '0'
    in 'a'..'f' -> c - 'a' + 10
    in 'A'..'F' -> c - 'A' + 10
    else -> -1
}

/** Décode les `%XX` d'une adresse. Contrairement à URLDecoder, `+` reste `+` (un mot de passe peut en contenir). */
fun percentDecode(s: String): String {
    if (!s.contains('%')) return s
    val out = java.io.ByteArrayOutputStream(s.length)
    var i = 0
    while (i < s.length) {
        val hi = if (s[i] == '%' && i + 2 < s.length + 0) hexDigit(s[i + 1]) else -1
        val lo = if (hi >= 0) hexDigit(s[i + 2]) else -1
        if (lo >= 0) {
            out.write(hi * 16 + lo)
            i += 3
        } else {
            val end = if (Character.isHighSurrogate(s[i]) && i + 1 < s.length) i + 2 else i + 1
            out.write(s.substring(i, end).toByteArray(Charsets.UTF_8))
            i = end
        }
    }
    return String(out.toByteArray(), Charsets.UTF_8)
}

/**
 * Paramètres de connexion JDBC tirés d'une adresse `postgres://utilisateur:motdepasse@hôte:5432/base?sslmode=require`.
 * [identity] : hôte et base, pour reconnaître la même base sous une adresse réécrite (mot de passe changé…).
 */
data class DbConfig(
    val jdbcUrl: String,
    val user: String,
    val password: String,
    val ssl: Boolean,
    val display: String,
    val identity: String = "",
)

fun parseDbUrl(raw: String): DbConfig? {
    val url = raw.trim()
    val rest = url.removePrefix("postgresql://").removePrefix("postgres://").takeIf { it != url } ?: return null
    val at = rest.lastIndexOf('@')
    val (creds, hostPart) = if (at >= 0) rest.substring(0, at) to rest.substring(at + 1) else "" to rest
    val user = percentDecode(creds.substringBefore(':'))
    val password = if (creds.contains(':')) percentDecode(creds.substringAfter(':')) else ""
    val query = hostPart.substringAfter('?', "")
    val hostDb = hostPart.substringBefore('?')
    val host = hostDb.substringBefore('/')
    val db = hostDb.substringAfter('/', "postgres").ifEmpty { "postgres" }
    if (host.isEmpty()) return null
    val params = query.split('&').filter { it.contains('=') }.associate { it.substringBefore('=') to it.substringAfter('=') }
    // Comme libpq : `require` chiffre la connexion sans vérifier le certificat (protège de l'écoute, pas d'un serveur
    // usurpé) ; `verify-ca` vérifie le certificat avec les autorités du système, et `verify-full` vérifie en plus
    // qu'il est bien celui de l'hôte. La fabrique est toujours nommée : celle par défaut du pilote (LibPQFactory)
    // lit ~/.postgresql, absent d'Android.
    val unverified = "sslfactory=org.postgresql.ssl.NonValidatingFactory"
    val verified = "sslfactory=org.postgresql.ssl.DefaultJavaSSLFactory"
    val sslParams = when (val mode = params["sslmode"] ?: "disable") {
        "disable" -> "sslmode=disable"
        "allow", "prefer", "require" -> "sslmode=$mode&$unverified"
        "verify-ca", "verify-full" -> "sslmode=$mode&$verified"
        else -> return null // mode inconnu : on ne devine pas un niveau de sécurité
    }
    val ssl = sslParams != "sslmode=disable"
    val jdbc = "jdbc:postgresql://$host/$db?$sslParams&ApplicationName=Mnemo%20Android&connectTimeout=10&socketTimeout=30&tcpKeepAlive=true"
    val display = "postgres://${if (user.isNotEmpty()) "$user@" else ""}$host/$db"
    val hostKey = host.lowercase().let { if (it.contains(':')) it else "$it:5432" }
    return DbConfig(jdbc, user, password, ssl, display, "$hostKey/$db")
}
