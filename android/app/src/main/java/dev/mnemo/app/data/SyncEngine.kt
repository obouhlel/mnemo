package dev.mnemo.app.data

import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.io.RandomAccessFile
import java.nio.channels.Channels
import java.sql.Connection
import java.sql.DriverManager
import java.sql.SQLException
import java.util.Properties
import java.util.UUID

/** Bilan d'une synchronisation. */
data class SyncReport(val pulled: Int = 0, val pushed: Int = 0, val conflicts: Int = 0, val files: Int = 0)

/**
 * Identité d'une base : son identifiant (`mnemo_meta.db_id`, posé par Mnemo sur ordinateur) s'il existe, et son
 * adresse (hôte, port, nom). Sert à ne pas confondre « la même base sous une adresse réécrite » et « une autre base ».
 */
data class DbIdentity(val addr: String, val id: String?) {
    fun sameAs(other: DbIdentity): Boolean = if (id != null && other.id != null) id == other.id else addr == other.addr
}

/**
 * Synchronisation de la copie locale avec la base PostgreSQL de Mnemo (même schéma que l'appli de bureau) :
 * envoie les modifications faites sur l'appareil (verrouillage optimiste par `version`, comme le bureau),
 * puis récupère ce qui a changé ailleurs. En cas de conflit, rien n'est perdu : le texte de l'appareil est
 * gardé dans une note « (conflit mobile …) » et la version de la base est reprise.
 *
 * Rien n'est noté « envoyé » sur l'appareil avant le COMMIT : les changements locaux sont préparés pendant la
 * transaction puis appliqués d'un coup après. Une synchronisation coupée est simplement refaite, et chaque étape
 * reconnaît ce qui est déjà dans la base.
 */
class SyncEngine(private val local: LocalDb, private val clientId: String) {

    /** Taille maximale d'une pièce jointe synchronisée : celle du bureau (MAX_FILE, src/db/vault_sync.rs). */
    private val maxFile = 50L * 1024 * 1024

    /** Les pièces jointes sont reçues par morceaux, jamais en entier en mémoire. */
    private val chunk = 1024 * 1024

    private class Tally(var pulled: Int = 0, var pushed: Int = 0, var conflicts: Int = 0, var files: Int = 0)

    fun connect(cfg: DbConfig): Connection {
        Class.forName("org.postgresql.Driver")
        val props = Properties().apply {
            setProperty("user", cfg.user)
            setProperty("password", cfg.password)
        }
        return DriverManager.getConnection(cfg.jdbcUrl, props)
    }

    /** Vérifie la connexion et le schéma ; renvoie un message d'erreur en français, ou null si tout va bien. */
    fun test(cfg: DbConfig): String? = try {
        connect(cfg).use { c ->
            if (tableExists(c, "notes")) null
            else "Base vide : ouvrez-la une première fois avec Mnemo sur ordinateur (Paramètres › Synchronisation)."
        }
    } catch (e: SQLException) {
        readable(e)
    } catch (e: Exception) {
        e.message ?: e.toString()
    }

    fun identity(cfg: DbConfig): DbIdentity = connect(cfg).use { c ->
        val id = if (!tableExists(c, "mnemo_meta")) null else c.createStatement().use { st ->
            st.executeQuery("SELECT value FROM mnemo_meta WHERE key = 'db_id'").use { rs -> if (rs.next()) rs.getString(1) else null }
        }
        DbIdentity(cfg.identity, id)
    }

    fun readable(e: Throwable): String {
        val m = e.message ?: e.toString()
        return when {
            m.contains("password authentication failed") -> "Mot de passe refusé par la base"
            m.contains("Connection refused") || m.contains("connect timed out") || m.contains("UnknownHost") ->
                "Base injoignable depuis cet appareil (adresse, réseau ou pare-feu)"
            m.contains("PKIX") || m.contains("CertPath") || m.contains("Trust anchor") || m.contains("hostname", true) && m.contains("verif", true) ->
                "Certificat de la base refusé (sslmode=verify-ca / verify-full)"
            else -> m
        }
    }

    /** [beforeCommit] : appelé juste avant le COMMIT (tests : simuler une coupure). */
    fun sync(cfg: DbConfig, beforeCommit: () -> Unit = {}): SyncReport = connect(cfg).use { c ->
        c.autoCommit = false
        val after = ArrayList<() -> Unit>() // changements de la copie locale, appliqués après le COMMIT
        val t = Tally()
        local.incomingDir.deleteRecursively()
        local.incomingDir.mkdirs()
        try {
            // marque les écritures de l'appareil (les autres instances voient qui a modifié)
            c.prepareStatement("SELECT set_config('app.client_id', ?, true)").use { it.setString(1, clientId); it.execute() }
            val refetch = pushNotes(c, after, t)
            pullNotes(c, refetch, after, t)
            pullFolders(c, after)
            syncFiles(c, after, t)
            pullBoard(c, after)
            beforeCommit()
            c.commit()
        } catch (e: Exception) {
            runCatching { c.rollback() }
            local.incomingDir.deleteRecursively()
            throw e
        }
        local.exclusive {
            after.forEach { it() }
            local.setMeta("last_sync", System.currentTimeMillis().toString())
        }
        local.incomingDir.deleteRecursively()
        SyncReport(t.pulled, t.pushed, t.conflicts, t.files)
    }

    // ------------------------------------------------------------------------------ notes

    private class ServerNote(val path: String, val content: String, val version: Int)

    private fun serverNote(c: Connection, id: UUID): ServerNote? =
        c.prepareStatement("SELECT path, content, version FROM notes WHERE id = ?").use { st ->
            st.setObject(1, id)
            st.executeQuery().use { rs -> if (rs.next()) ServerNote(rs.getString(1), rs.getString(2), rs.getInt(3)) else null }
        }

    /**
     * Exécute [block] ; si la base refuse la ligne (nom déjà pris, chemin invalide), seule cette écriture est
     * annulée (point de sauvegarde) et le résultat est null : une note refusée ne bloque pas les autres.
     */
    private fun <T : Any> unlessRejected(c: Connection, block: () -> T): T? {
        val sp = c.setSavepoint()
        return try {
            block().also { c.releaseSavepoint(sp) }
        } catch (e: SQLException) {
            if (e.sqlState != "23505" && e.sqlState != "23514") throw e
            c.rollback(sp)
            null
        }
    }

    /** Le chemin de la note, puis des chemins de repli « … (mobile) », « … (mobile 2) »… */
    private fun candidates(path: String): Sequence<String> {
        val base = if (isSafeNotePath(path)) path else newNotePath(path).getOrDefault("Sans titre")
        return sequenceOf(path) + (1..30).asSequence().map { if (it == 1) "$base (mobile)" else "$base (mobile $it)" }
    }

    /** Écrit la note avec le premier chemin que la base accepte ; renvoie (chemin retenu, version), version 0 = ligne absente. */
    private fun writeFree(c: Connection, n: NoteRow, write: (String) -> Int): Pair<String, Int> {
        for (path in candidates(n.path)) {
            val v = unlessRejected(c) { write(path) } ?: continue
            return path to v
        }
        throw SQLException("Aucun nom libre pour « ${n.path} »")
    }

    private fun insertNote(c: Connection, id: UUID, path: String, content: String, quiet: Boolean = false): Int =
        c.prepareStatement("INSERT INTO notes (id, path, content) VALUES (?, ?, ?) ${if (quiet) "ON CONFLICT DO NOTHING" else ""} RETURNING version").use { st ->
            st.setObject(1, id); st.setString(2, path); st.setString(3, content)
            st.executeQuery().use { rs -> if (rs.next()) rs.getInt(1) else 0 }
        }

    /** Envoie les notes modifiées ; renvoie celles dont la version de la base doit être reprise dans la foulée. */
    private fun pushNotes(c: Connection, after: MutableList<() -> Unit>, t: Tally): Set<String> {
        val refetch = HashSet<String>()
        for (n in local.dirtyNotes()) {
            val id = runCatching { UUID.fromString(n.id) }.getOrNull() ?: continue
            if (n.deleted) {
                if (n.version > 0) {
                    c.prepareStatement("DELETE FROM notes WHERE id = ? AND version = ?").use { st ->
                        st.setObject(1, id); st.setInt(2, n.version); st.executeUpdate()
                    }
                }
                // modifiée ailleurs entre-temps : la suppression est abandonnée, la note revient
                after += { local.removeDeleted(n.id) }
                refetch += n.id
                t.pushed++
                continue
            }
            val server = serverNote(c, id)
            when {
                // nouvelle note, ou supprimée ailleurs pendant qu'elle était modifiée ici : (re)créée
                server == null -> {
                    val (path, v) = writeFree(c, n) { insertNote(c, id, it, n.content) }
                    after += { local.settle(n, v, path) }
                    t.pushed++
                }
                // déjà dans la base : envoi précédent coupé après le COMMIT, même modification faite ailleurs,
                // ou modification annulée (taper puis effacer) : ce n'est pas un conflit
                server.content == n.content && server.path == n.path -> after += { local.settle(n, server.version) }
                server.version == n.version -> {
                    val (path, v) = writeFree(c, n) { p ->
                        c.prepareStatement("UPDATE notes SET path = ?, content = ? WHERE id = ? AND version = ? RETURNING version").use { st ->
                            st.setString(1, p); st.setString(2, n.content); st.setObject(3, id); st.setInt(4, n.version)
                            st.executeQuery().use { rs -> if (rs.next()) rs.getInt(1) else 0 }
                        }
                    }
                    if (v > 0) {
                        after += { local.settle(n, v, path) }
                        t.pushed++
                    } else {
                        conflict(c, n, after, t, refetch) // modifiée ailleurs à l'instant
                    }
                }
                else -> conflict(c, n, after, t, refetch)
            }
        }
        return refetch
    }

    /** Modifiée ailleurs depuis : le texte de l'appareil devient une note à part, la note reprend la base. */
    private fun conflict(c: Connection, n: NoteRow, after: MutableList<() -> Unit>, t: Tally, refetch: MutableSet<String>) {
        // déjà mise de côté par une synchronisation coupée après le COMMIT ?
        val kept = c.prepareStatement("SELECT 1 FROM notes WHERE starts_with(path, ?) AND content = ? LIMIT 1").use { st ->
            st.setString(1, "${n.path} (conflit mobile "); st.setString(2, n.content)
            st.executeQuery().use { it.next() }
        }
        if (!kept) {
            val base = conflictName(if (isSafeNotePath(n.path)) n.path else "Sans titre")
            val saved = (1..50).any { i ->
                unlessRejected(c) { insertNote(c, UUID.randomUUID(), if (i == 1) base else "$base $i", n.content, quiet = true) }.let { it != null && it > 0 }
            }
            // sans copie dans la base, la note reste telle quelle sur l'appareil
            if (!saved) throw SQLException("Copie de conflit impossible pour « ${n.path} »")
        }
        after += { local.resolveConflict(n) }
        refetch += n.id
        t.conflicts++
    }

    private class Pulled(val id: String, val path: String, val content: String, val version: Int, val updatedAt: Long)

    private fun pullNotes(c: Connection, refetch: Set<String>, after: MutableList<() -> Unit>, t: Tally) {
        val remote = HashMap<String, Int>()
        c.createStatement().use { st ->
            st.executeQuery("SELECT id::text, version FROM notes").use { rs ->
                while (rs.next()) remote[rs.getString(1)] = rs.getInt(2)
            }
        }
        val mine = local.noteStates() // id → (version, à envoyer)
        val toFetch = remote.filter { (id, v) -> id in refetch || mine[id].let { it == null || (!it.second && it.first != v) } }.keys.toList()
        val rows = ArrayList<Pulled>()
        for (ids in toFetch.chunked(200)) {
            c.prepareStatement(
                "SELECT id::text, path, content, version, (extract(epoch FROM updated_at) * 1000)::bigint FROM notes WHERE id::text = ANY(?)",
            ).use { st ->
                st.setArray(1, c.createArrayOf("text", ids.toTypedArray()))
                st.executeQuery().use { rs ->
                    while (rs.next()) {
                        // un chemin invalide (« .. », absolu…) n'entre pas sur l'appareil
                        if (isSafeNotePath(rs.getString(2))) rows += Pulled(rs.getString(1), rs.getString(2), rs.getString(3), rs.getInt(4), rs.getLong(5))
                    }
                }
            }
        }
        // supprimées ailleurs (et pas modifiées ici)
        val gone = mine.filter { (id, s) -> !s.second && s.first != 0 && id !in remote }.keys
        after += {
            // une note modifiée sur l'appareil pendant la synchronisation n'est ni remplacée ni retirée
            rows.forEach { if (local.putRemote(it.id, it.path, it.content, it.version, it.updatedAt)) t.pulled++ }
            gone.forEach { if (local.removeIfClean(it)) t.pulled++ }
        }
    }

    private fun pullFolders(c: Connection, after: MutableList<() -> Unit>) {
        val folders = mutableListOf<String>()
        c.createStatement().use { st ->
            st.executeQuery("SELECT path FROM folders").use { rs -> while (rs.next()) folders += rs.getString(1) }
        }
        after += { local.replaceFolders(folders.filter(::isSafeRelPath)) }
    }

    // ------------------------------------------------------------------------------ pièces jointes

    private fun tableExists(c: Connection, name: String): Boolean =
        c.prepareStatement("SELECT to_regclass(?) IS NOT NULL").use { st ->
            st.setString(1, "public.$name")
            st.executeQuery().use { rs -> rs.next() && rs.getBoolean(1) }
        }

    private fun upload(c: Connection, path: String, data: java.io.InputStream, size: Long, hash: String) {
        c.prepareStatement(
            """INSERT INTO files (path, data, hash, size) VALUES (?, ?, ?, ?)
               ON CONFLICT (path) DO UPDATE SET data = EXCLUDED.data, hash = EXCLUDED.hash, size = EXCLUDED.size, updated_at = now()""",
        ).use { st ->
            st.setString(1, path); st.setBinaryStream(2, data, size.toInt()); st.setString(3, hash); st.setLong(4, size)
            st.executeUpdate()
        }
    }

    /**
     * Télécharge une pièce jointe par morceaux dans un fichier temporaire (déplacé dans le coffre après le COMMIT).
     * Renvoie null si le contenu a changé pendant la lecture : il sera repris à la prochaine synchronisation.
     */
    private fun download(c: Connection, path: String, hash: String): File? {
        val tmp = File(local.incomingDir, UUID.randomUUID().toString())
        val fnv = Fnv()
        tmp.outputStream().use { out ->
            c.prepareStatement("SELECT substring(data FROM ? FOR ?) FROM files WHERE path = ?").use { st ->
                var offset = 1
                while (true) {
                    st.setInt(1, offset); st.setInt(2, chunk); st.setString(3, path)
                    val part = st.executeQuery().use { rs -> if (rs.next()) rs.getBytes(1) else null } ?: break
                    out.write(part)
                    fnv.update(part)
                    if (part.size < chunk) break
                    offset += chunk
                }
            }
        }
        if (fnv.hex() == hash) return tmp
        // l'empreinte ne correspond pas : fichier remplacé pendant la lecture (abandon), ou empreinte fausse dans la base
        val now = c.prepareStatement("SELECT hash FROM files WHERE path = ?").use { st ->
            st.setString(1, path)
            st.executeQuery().use { rs -> if (rs.next()) rs.getString(1) else null }
        }
        if (now == hash) {
            val again = c.prepareStatement("SELECT length(data) FROM files WHERE path = ?").use { st ->
                st.setString(1, path)
                st.executeQuery().use { rs -> if (rs.next()) rs.getLong(1) else -1 }
            }
            if (again == tmp.length()) return tmp
        }
        tmp.delete()
        return null
    }

    private fun syncFiles(c: Connection, after: MutableList<() -> Unit>, t: Tally) {
        if (!tableExists(c, "files")) return
        val remote = HashMap<String, Pair<String, Long>>()
        c.createStatement().use { st ->
            st.executeQuery("SELECT path, hash, size FROM files").use { rs ->
                // un chemin qui sortirait du coffre (« ../ », absolu, caché) est ignoré
                while (rs.next()) if (isSafeRelPath(rs.getString(1))) remote[rs.getString(1)] = rs.getString(2) to rs.getLong(3)
            }
        }
        // état des fichiers tel qu'il sera après le COMMIT (la réception ci-dessous en tient compte)
        val mine = local.files(withDeleted = true).associateByTo(HashMap()) { it.path }
        // envoi des fichiers faits ou modifiés ici
        for (f in mine.values.filter { it.dirty }) {
            if (f.deleted) {
                val n = c.prepareStatement("DELETE FROM files WHERE path = ? AND hash = ?").use { st ->
                    st.setString(1, f.path); st.setString(2, f.hash ?: ""); st.executeUpdate()
                }
                after += { local.removeDeletedFile(f.path) }
                mine.remove(f.path)
                if (n > 0) remote.remove(f.path) // sinon modifié ailleurs : il revient
                t.files++
                continue
            }
            val file = local.localFile(f.path) ?: continue
            if (file.length() > maxFile) continue // trop gros pour la base (comme sur ordinateur) : reste sur l'appareil
            // le fichier est ouvert une seule fois : empreinte et envoi portent sur le même contenu, même s'il est
            // réenregistré pendant ce temps (l'enregistrement remplace le fichier, il ne le modifie pas)
            RandomAccessFile(file, "r").use { raf ->
                val ch = raf.channel
                val size = ch.size()
                val h = fileHash(Channels.newInputStream(ch))
                val remoteHash = remote[f.path]?.first
                if (remoteHash != null && remoteHash != f.hash && remoteHash != h) {
                    // modifié des deux côtés : la version de l'appareil est envoyée à côté, sans écraser une copie
                    // précédente (sauf si c'est déjà ce contenu : envoi coupé après le COMMIT)
                    val (base, ext) = fileConflictBase(f.path)
                    val copy = firstFree(base, ext) { remote[it].let { r -> r != null && r.first != h } }
                    if (remote[copy] == null) {
                        ch.position(0)
                        upload(c, copy, Channels.newInputStream(ch), size, h)
                        remote[copy] = h to size
                    }
                    after += { local.releaseFile(f.path, h, redownload = true) } // la version de la base est reprise
                    mine[f.path] = f.copy(dirty = false, localHash = null)
                } else {
                    if (remoteHash != h) {
                        ch.position(0)
                        upload(c, f.path, Channels.newInputStream(ch), size, h)
                    }
                    after += { local.settleFile(f.path, h, h) }
                    remote[f.path] = h to size
                    mine[f.path] = FileRow(f.path, h, size, h, dirty = false, deleted = false)
                }
            }
            t.files++
        }
        // réception
        for ((path, info) in remote) {
            val (hash, size) = info
            val row = mine[path]
            if (row != null && row.dirty) continue
            val needData = row?.localHash != hash || local.localFile(path) == null
            // au-delà de la limite du bureau : listé, pas téléchargé
            val tmp = if (needData && size <= maxFile) download(c, path, hash) else null
            if (tmp != null || row == null || row.hash != hash || row.size != size) {
                after += { if (local.installRemote(path, hash, size, tmp) && tmp != null) t.files++ }
            }
        }
        // supprimés ailleurs (et pas modifiés ici entre-temps)
        mine.values.filter { !it.dirty && it.hash != null && it.path !in remote }.forEach { f ->
            after += { if (local.removeFileIfClean(f.path)) t.files++ }
        }
    }

    // ------------------------------------------------------------------------------ tableau de bord (lecture)

    private fun pullBoard(c: Connection, after: MutableList<() -> Unit>) {
        if (!tableExists(c, "projects")) return
        val board = JSONObject()
        val projects = JSONArray()
        c.createStatement().use { st ->
            st.executeQuery("SELECT id::text, name, coalesce(github_repo, '') FROM projects ORDER BY position, lower(name)").use { rs ->
                while (rs.next()) projects.put(JSONObject().put("id", rs.getString(1)).put("name", rs.getString(2)).put("repo", rs.getString(3)))
            }
        }
        val hasPr = c.prepareStatement(
            "SELECT 1 FROM information_schema.columns WHERE table_name = 'tickets' AND column_name = 'github_pr'",
        ).use { st -> st.executeQuery().use { it.next() } }
        val tickets = JSONArray()
        c.createStatement().use { st ->
            st.executeQuery(
                """SELECT id::text, project_id::text, number, title, body, status, tags, priority, github_number,
                          coalesce(github_state, ''), ${if (hasPr) "github_pr" else "false"}
                   FROM tickets ORDER BY priority DESC, position, number""",
            ).use { rs ->
                while (rs.next()) {
                    @Suppress("UNCHECKED_CAST")
                    val tags = (rs.getArray(7)?.array as? Array<String>)?.toList().orEmpty()
                    tickets.put(
                        JSONObject().put("id", rs.getString(1)).put("project", rs.getString(2)).put("number", rs.getInt(3))
                            .put("title", rs.getString(4)).put("body", rs.getString(5)).put("status", rs.getString(6))
                            .put("tags", JSONArray(tags)).put("priority", rs.getInt(8))
                            .put("github", rs.getObject(9)?.let { (it as Number).toInt() } ?: JSONObject.NULL)
                            .put("state", rs.getString(10)).put("pr", rs.getBoolean(11)),
                    )
                }
            }
        }
        board.put("projects", projects).put("tickets", tickets)
        after += { local.setMeta("board", board.toString()) }
    }
}
