package dev.mnemo.app.data

import org.json.JSONArray
import org.json.JSONObject
import java.sql.Connection
import java.sql.DriverManager
import java.sql.SQLException
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.Properties
import java.util.UUID

/** Bilan d'une synchronisation. */
data class SyncReport(val pulled: Int = 0, val pushed: Int = 0, val conflicts: Int = 0, val files: Int = 0)

/**
 * Synchronisation de la copie locale avec la base PostgreSQL de Mnemo (même schéma que l'appli de bureau) :
 * envoie les modifications faites sur l'appareil (verrouillage optimiste par `version`, comme le bureau),
 * puis récupère ce qui a changé ailleurs. En cas de conflit, rien n'est perdu : le texte de l'appareil est
 * gardé dans une note « (conflit mobile …) » et la version de la base est reprise.
 */
class SyncEngine(private val local: LocalDb, private val clientId: String) {

    /** Taille maximale d'une pièce jointe téléchargée automatiquement. */
    private val maxDownload = 20L * 1024 * 1024

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
            c.createStatement().use { st ->
                st.executeQuery("SELECT to_regclass('public.notes') IS NOT NULL").use { rs ->
                    rs.next()
                    if (!rs.getBoolean(1)) "Base vide : ouvrez-la une première fois avec Mnemo sur ordinateur (Paramètres › Synchronisation)."
                    else null
                }
            }
        }
    } catch (e: SQLException) {
        readable(e)
    } catch (e: Exception) {
        e.message ?: e.toString()
    }

    fun readable(e: Throwable): String {
        val m = e.message ?: e.toString()
        return when {
            m.contains("password authentication failed") -> "Mot de passe refusé par la base"
            m.contains("Connection refused") || m.contains("connect timed out") || m.contains("UnknownHost") ->
                "Base injoignable depuis cet appareil (adresse, réseau ou pare-feu)"
            else -> m
        }
    }

    fun sync(cfg: DbConfig): SyncReport = connect(cfg).use { c ->
        c.autoCommit = false
        try {
            // marque les écritures de l'appareil (les autres instances voient qui a modifié)
            c.prepareStatement("SELECT set_config('app.client_id', ?, true)").use { it.setString(1, clientId); it.execute() }
            var r = pushNotes(c)
            r = r.copy(pulled = pullNotes(c))
            pullFolders(c)
            r = r.copy(files = syncFiles(c))
            pullBoard(c)
            c.commit()
            local.setMeta("last_sync", System.currentTimeMillis().toString())
            r
        } catch (e: Exception) {
            runCatching { c.rollback() }
            throw e
        }
    }

    // ------------------------------------------------------------------------------ notes

    private fun conflictPath(path: String) =
        "$path (conflit mobile ${SimpleDateFormat("yyyy-MM-dd HHmm", Locale.FRANCE).format(Date())})"

    private fun insertNote(c: Connection, id: UUID, path: String, content: String): Int? =
        c.prepareStatement("INSERT INTO notes (id, path, content) VALUES (?, ?, ?) ON CONFLICT DO NOTHING RETURNING version").use { st ->
            st.setObject(1, id); st.setString(2, path); st.setString(3, content)
            st.executeQuery().use { rs -> if (rs.next()) rs.getInt(1) else null }
        }

    private fun pushNotes(c: Connection): SyncReport {
        var pushed = 0
        var conflicts = 0
        for (n in local.notes(withDeleted = true).filter { it.dirty }) {
            val id = UUID.fromString(n.id)
            when {
                n.deleted -> {
                    if (n.version > 0) {
                        c.prepareStatement("DELETE FROM notes WHERE id = ? AND version = ?").use { st ->
                            st.setObject(1, id); st.setInt(2, n.version); st.executeUpdate()
                        }
                    }
                    // modifiée ailleurs entre-temps : la suppression est abandonnée, la note revient au passage suivant
                    local.removeNote(n.id)
                    pushed++
                }
                n.version == 0 -> {
                    val v = insertNote(c, id, n.path, n.content)
                        ?: insertNote(c, id, "${n.path} (mobile)", n.content) // nom déjà pris dans la base
                    if (v != null) {
                        local.markClean(n.id, v)
                        pushed++
                    }
                }
                else -> {
                    val v = c.prepareStatement(
                        "UPDATE notes SET path = ?, content = ? WHERE id = ? AND version = ? RETURNING version",
                    ).use { st ->
                        st.setString(1, n.path); st.setString(2, n.content); st.setObject(3, id); st.setInt(4, n.version)
                        st.executeQuery().use { rs -> if (rs.next()) rs.getInt(1) else null }
                    }
                    if (v != null) {
                        local.markClean(n.id, v)
                        pushed++
                    } else {
                        // modifiée ailleurs depuis : le texte de l'appareil devient une note à part, la note reprend la base
                        insertNote(c, UUID.randomUUID(), conflictPath(n.path), n.content)
                        local.markClean(n.id, -1)
                        conflicts++
                    }
                }
            }
        }
        return SyncReport(pushed = pushed, conflicts = conflicts)
    }

    private fun pullNotes(c: Connection): Int {
        val remote = HashMap<String, Int>()
        c.createStatement().use { st ->
            st.executeQuery("SELECT id::text, version FROM notes").use { rs ->
                while (rs.next()) remote[rs.getString(1)] = rs.getInt(2)
            }
        }
        val mine = local.notes(withDeleted = true).associateBy { it.id }
        val toFetch = remote.filter { (id, v) -> mine[id].let { it == null || (!it.dirty && it.version != v) } }.keys.toList()
        for (chunk in toFetch.chunked(200)) {
            c.prepareStatement(
                "SELECT id::text, path, content, version, (extract(epoch FROM updated_at) * 1000)::bigint FROM notes WHERE id::text = ANY(?)",
            ).use { st ->
                st.setArray(1, c.createArrayOf("text", chunk.toTypedArray()))
                st.executeQuery().use { rs ->
                    while (rs.next()) local.putRemote(rs.getString(1), rs.getString(2), rs.getString(3), rs.getInt(4), rs.getLong(5))
                }
            }
        }
        // supprimées ailleurs (et pas modifiées ici)
        mine.values.filter { !it.dirty && it.version != 0 && it.id !in remote }.forEach { local.removeNote(it.id) }
        return toFetch.size
    }

    private fun pullFolders(c: Connection) {
        val folders = mutableListOf<String>()
        c.createStatement().use { st ->
            st.executeQuery("SELECT path FROM folders").use { rs -> while (rs.next()) folders += rs.getString(1) }
        }
        local.replaceFolders(folders)
    }

    // ------------------------------------------------------------------------------ pièces jointes

    private fun tableExists(c: Connection, name: String): Boolean =
        c.prepareStatement("SELECT to_regclass(?) IS NOT NULL").use { st ->
            st.setString(1, "public.$name")
            st.executeQuery().use { rs -> rs.next() && rs.getBoolean(1) }
        }

    private fun upload(c: Connection, path: String, bytes: ByteArray, hash: String) {
        c.prepareStatement(
            """INSERT INTO files (path, data, hash, size) VALUES (?, ?, ?, ?)
               ON CONFLICT (path) DO UPDATE SET data = EXCLUDED.data, hash = EXCLUDED.hash, size = EXCLUDED.size, updated_at = now()""",
        ).use { st ->
            st.setString(1, path); st.setBytes(2, bytes); st.setString(3, hash); st.setLong(4, bytes.size.toLong())
            st.executeUpdate()
        }
    }

    private fun syncFiles(c: Connection): Int {
        if (!tableExists(c, "files")) return 0
        val remote = HashMap<String, Pair<String, Long>>()
        c.createStatement().use { st ->
            st.executeQuery("SELECT path, hash, size FROM files").use { rs ->
                while (rs.next()) remote[rs.getString(1)] = rs.getString(2) to rs.getLong(3)
            }
        }
        var count = 0
        // envoi des fichiers faits ou modifiés ici
        for (f in local.files(withDeleted = true).filter { it.dirty }) {
            if (f.deleted) {
                c.prepareStatement("DELETE FROM files WHERE path = ? AND hash = ?").use { st ->
                    st.setString(1, f.path); st.setString(2, f.hash ?: ""); st.executeUpdate()
                }
                local.removeFile(f.path)
                remote.remove(f.path)
                count++
                continue
            }
            val file = local.fileOnDisk(f.path)
            if (!file.exists()) continue
            val bytes = file.readBytes()
            val h = fileHash(bytes)
            val remoteHash = remote[f.path]?.first
            if (remoteHash != null && remoteHash != f.hash && remoteHash != h) {
                // modifié des deux côtés : la version de l'appareil est envoyée à côté
                val dot = f.path.lastIndexOf('.')
                val copy = if (dot > 0) "${f.path.substring(0, dot)} (conflit mobile)${f.path.substring(dot)}" else "${f.path} (conflit mobile)"
                upload(c, copy, bytes, h)
                local.putFile(f.copy(dirty = false, localHash = null)) // la version de la base sera retéléchargée
            } else {
                upload(c, f.path, bytes, h)
                local.putFile(FileRow(f.path, h, bytes.size.toLong(), h, dirty = false, deleted = false))
                remote[f.path] = h to bytes.size.toLong()
            }
            count++
        }
        // réception
        val mine = local.files(withDeleted = true).associateBy { it.path }
        for ((path, info) in remote) {
            val (hash, size) = info
            val row = mine[path]
            if (row != null && row.dirty) continue
            val needData = row?.localHash != hash || !local.fileOnDisk(path).exists()
            var localHash = row?.localHash
            if (needData && size <= maxDownload) {
                c.prepareStatement("SELECT data FROM files WHERE path = ?").use { st ->
                    st.setString(1, path)
                    st.executeQuery().use { rs ->
                        if (rs.next()) {
                            val f = local.fileOnDisk(path)
                            f.parentFile?.mkdirs()
                            f.writeBytes(rs.getBytes(1))
                            localHash = hash
                            count++
                        }
                    }
                }
            }
            if (row == null || row.hash != hash || row.localHash != localHash) {
                local.putFile(FileRow(path, hash, size, localHash, dirty = false, deleted = false))
            }
        }
        // supprimés ailleurs
        mine.values.filter { !it.dirty && it.hash != null && it.path !in remote }.forEach { local.removeFile(it.path); count++ }
        return count
    }

    // ------------------------------------------------------------------------------ tableau de bord (lecture)

    private fun pullBoard(c: Connection) {
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
        local.setMeta("board", board.toString())
    }
}
