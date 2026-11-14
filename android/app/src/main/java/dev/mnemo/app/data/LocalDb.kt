package dev.mnemo.app.data

import android.content.ContentValues
import android.content.Context
import android.database.sqlite.SQLiteDatabase
import android.database.sqlite.SQLiteOpenHelper
import java.io.File

data class NoteRow(
    val id: String,
    val path: String,
    val content: String,
    /** version connue dans la base ; 0 = jamais envoyée */
    val version: Int,
    val updatedAt: Long,
    val dirty: Boolean,
    val deleted: Boolean,
)

/** Une note sans son texte (listes, liens) : rien de lourd à charger. */
data class NoteMeta(val id: String, val path: String, val dirty: Boolean)

data class SearchHit(val id: String, val path: String, val snippet: String)

data class FileRow(
    val path: String,
    /** empreinte dans la base (null : pas encore envoyé) */
    val hash: String?,
    val size: Long,
    /** empreinte de la copie locale (null : pas encore téléchargé) */
    val localHash: String?,
    val dirty: Boolean,
    val deleted: Boolean,
)

/**
 * Copie locale de la base (SQLite) : l'appli marche hors ligne, et les modifications (`dirty`) sont envoyées à la
 * prochaine synchronisation. Le contenu des pièces jointes est dans [vaultDir], au même chemin que sur ordinateur.
 *
 * L'écran et la synchronisation écrivent en même temps : chaque opération est atomique, et celles de la
 * synchronisation ne s'appliquent que si la ligne est encore dans l'état qu'elle a lu (rien d'enregistré entre-temps).
 */
class LocalDb(context: Context, name: String = "mnemo.db") :
    SQLiteOpenHelper(context, name, null, 1) {

    val vaultDir: File = File(context.filesDir, "vault").apply { mkdirs() }

    /** Téléchargements en cours : hors du coffre, déplacés d'un coup une fois complets. */
    val incomingDir: File = File(context.filesDir, "incoming")

    private val vaultRoot: String = vaultDir.canonicalPath + File.separator

    override fun onCreate(db: SQLiteDatabase) {
        db.execSQL(
            """CREATE TABLE notes (id TEXT PRIMARY KEY, path TEXT NOT NULL, content TEXT NOT NULL,
               version INTEGER NOT NULL DEFAULT 0, updated_at INTEGER NOT NULL DEFAULT 0,
               dirty INTEGER NOT NULL DEFAULT 0, deleted INTEGER NOT NULL DEFAULT 0)""",
        )
        db.execSQL(
            """CREATE TABLE files (path TEXT PRIMARY KEY, hash TEXT, size INTEGER NOT NULL DEFAULT 0, local_hash TEXT,
               dirty INTEGER NOT NULL DEFAULT 0, deleted INTEGER NOT NULL DEFAULT 0)""",
        )
        db.execSQL("CREATE TABLE folders (path TEXT PRIMARY KEY)")
        db.execSQL("CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT)")
    }

    override fun onUpgrade(db: SQLiteDatabase, oldVersion: Int, newVersion: Int) {}

    /** Plusieurs écritures en une fois : toutes, ou aucune. */
    fun <T> tx(block: () -> T): T {
        val db = writableDatabase
        db.beginTransaction()
        try {
            val r = block()
            db.setTransactionSuccessful()
            return r
        } finally {
            db.endTransaction()
        }
    }

    /** Comme [tx], pour un lot qui touche aussi aux fichiers (même ordre de verrous que les écritures de fichiers). */
    @Synchronized
    fun <T> exclusive(block: () -> T): T = tx(block)

    private fun update(sql: String, vararg args: Any?): Int =
        writableDatabase.compileStatement(sql).use { st ->
            args.forEachIndexed { i, a ->
                when (a) {
                    null -> st.bindNull(i + 1)
                    is String -> st.bindString(i + 1, a)
                    is Boolean -> st.bindLong(i + 1, if (a) 1 else 0)
                    is Number -> st.bindLong(i + 1, a.toLong())
                    else -> error("type non géré")
                }
            }
            st.executeUpdateDelete()
        }

    // ---------------------------------------------------------------- notes

    private fun note(c: android.database.Cursor) = NoteRow(
        c.getString(0), c.getString(1), c.getString(2), c.getInt(3), c.getLong(4), c.getInt(5) != 0, c.getInt(6) != 0,
    )

    private val noteCols = "id, path, content, version, updated_at, dirty, deleted"

    /** Toutes les notes avec leur texte, supprimées comprises si [withDeleted]. */
    fun notes(withDeleted: Boolean = false): List<NoteRow> =
        readableDatabase.rawQuery("SELECT $noteCols FROM notes ${if (withDeleted) "" else "WHERE deleted = 0"}", null)
            .use { c -> buildList { while (c.moveToNext()) add(note(c)) } }.sortedBy { pathKey(it.path) }

    /** Les notes sans leur texte, triées par chemin (sans casse, accents compris). */
    fun noteList(): List<NoteMeta> =
        readableDatabase.rawQuery("SELECT id, path, dirty FROM notes WHERE deleted = 0", null)
            .use { c -> buildList { while (c.moveToNext()) add(NoteMeta(c.getString(0), c.getString(1), c.getInt(2) != 0)) } }
            .sortedBy { pathKey(it.path) }

    /** Identifiant, version et état de chaque note (supprimées comprises), pour la synchronisation. */
    fun noteStates(): Map<String, Pair<Int, Boolean>> =
        readableDatabase.rawQuery("SELECT id, version, dirty FROM notes", null)
            .use { c -> buildMap { while (c.moveToNext()) put(c.getString(0), c.getInt(1) to (c.getInt(2) != 0)) } }

    /** Notes à envoyer (modifiées ou supprimées sur l'appareil). */
    fun dirtyNotes(): List<NoteRow> =
        readableDatabase.rawQuery("SELECT $noteCols FROM notes WHERE dirty = 1", null)
            .use { c -> buildList { while (c.moveToNext()) add(note(c)) } }

    fun note(id: String): NoteRow? =
        readableDatabase.rawQuery("SELECT $noteCols FROM notes WHERE id = ?", arrayOf(id)).use { c ->
            if (c.moveToFirst()) note(c) else null
        }

    /** Note à ce chemin, sans tenir compte de la casse (comparaison Unicode, pas le `lower()` ASCII de SQLite). */
    fun noteByPath(path: String): NoteRow? {
        val key = pathKey(path)
        return noteList().firstOrNull { pathKey(it.path) == key }?.let { note(it.id) }
    }

    /**
     * Note reçue de la base : remplace la copie locale, sauf si elle a été modifiée sur l'appareil entre-temps
     * (elle sera alors envoyée, ou mise de côté comme conflit, à la prochaine synchronisation). Renvoie vrai si appliquée.
     */
    fun putRemote(id: String, path: String, content: String, version: Int, updatedAt: Long): Boolean = tx {
        val values = ContentValues().apply {
            put("id", id); put("path", path); put("content", content); put("version", version)
            put("updated_at", updatedAt); put("dirty", 0); put("deleted", 0)
        }
        writableDatabase.update("notes", values, "id = ? AND dirty = 0", arrayOf(id)) > 0 ||
            writableDatabase.insertWithOnConflict("notes", null, values, SQLiteDatabase.CONFLICT_IGNORE) != -1L
    }

    /** Écrit une note telle quelle, à envoyer (création, tests). La version connue de la base est gardée. */
    fun saveLocal(id: String, path: String, content: String): Unit = tx {
        val existing = note(id)
        writableDatabase.insertWithOnConflict(
            "notes", null,
            ContentValues().apply {
                put("id", id); put("path", path); put("content", content); put("version", existing?.version ?: 0)
                put("updated_at", System.currentTimeMillis()); put("dirty", 1); put("deleted", 0)
            },
            SQLiteDatabase.CONFLICT_REPLACE,
        )
    }

    /**
     * Enregistre le texte saisi dans l'éditeur. [base] est le texte que l'éditeur avait chargé : si la note a changé
     * depuis (reçue d'un autre appareil pendant la saisie), elle n'est pas écrasée et la saisie part dans une note
     * « (conflit mobile …) ». Renvoie l'identifiant de la note qui contient maintenant [content].
     */
    fun saveEdit(id: String, path: String, base: String?, content: String): String = tx {
        val row = note(id)
        when {
            // supprimée ailleurs pendant la saisie : le texte est gardé, la note est recréée
            row == null -> id.also { saveLocal(id, path, content) }
            row.deleted -> id // supprimée ici : rien à garder
            row.content == content -> id
            base != null && row.content != base -> {
                val taken = noteList().mapTo(HashSet()) { pathKey(it.path) }
                val copy = java.util.UUID.randomUUID().toString()
                saveLocal(copy, firstFree(conflictName(row.path)) { pathKey(it) in taken }, content)
                copy
            }
            else -> id.also {
                update("UPDATE notes SET content = ?, dirty = 1, updated_at = ? WHERE id = ?", content, System.currentTimeMillis(), id)
            }
        }
    }

    fun rename(id: String, path: String) {
        update("UPDATE notes SET path = ?, dirty = 1, updated_at = ? WHERE id = ? AND deleted = 0", path, System.currentTimeMillis(), id)
    }

    fun markDeleted(id: String) {
        update("UPDATE notes SET deleted = 1, dirty = 1 WHERE id = ?", id)
    }

    /**
     * Après l'envoi de [pushed] : la base est à la [version] donnée. La note n'est marquée « à jour » que si elle
     * est restée telle qu'envoyée ; si elle a été modifiée pendant l'envoi, elle reste à envoyer (avec la nouvelle
     * version, pour que le prochain envoi ne soit pas pris pour un conflit). [newPath] : chemin retenu par la base
     * quand celui de l'appareil était pris ou refusé.
     */
    fun settle(pushed: NoteRow, version: Int, newPath: String? = null) {
        update(
            """UPDATE notes SET version = ?,
                 dirty = CASE WHEN content = ? AND path = ? AND deleted = ? THEN 0 ELSE dirty END,
                 path = CASE WHEN path = ? THEN ? ELSE path END
               WHERE id = ?""",
            version, pushed.content, pushed.path, pushed.deleted, pushed.path, newPath ?: pushed.path, pushed.id,
        )
    }

    /**
     * Conflit réglé dans la base (le texte de [pushed] y est dans une note à part) : la note peut reprendre la
     * version de la base, sauf si elle a encore changé sur l'appareil. Renvoie vrai si elle est prête à être remplacée.
     */
    fun resolveConflict(pushed: NoteRow): Boolean = update(
        "UPDATE notes SET dirty = 0, version = -1 WHERE id = ? AND content = ? AND path = ? AND deleted = 0",
        pushed.id, pushed.content, pushed.path,
    ) > 0

    /** Suppression envoyée : la ligne disparaît (sauf si la note a été recréée entre-temps). */
    fun removeDeleted(id: String) {
        update("DELETE FROM notes WHERE id = ? AND deleted = 1", id)
    }

    /** Note supprimée ailleurs : retirée, sauf si elle a été modifiée ici entre-temps. */
    fun removeIfClean(id: String): Boolean = update("DELETE FROM notes WHERE id = ? AND dirty = 0", id) > 0

    /** Recherche dans les chemins et les textes, sans casse ; à appeler hors du fil principal. */
    fun search(query: String, limit: Int = 200): List<SearchHit> {
        val q = query.trim()
        if (q.isEmpty()) return emptyList()
        return readableDatabase.rawQuery("SELECT id, path, content FROM notes WHERE deleted = 0", null).use { c ->
            buildList {
                while (c.moveToNext() && size < limit) {
                    val path = c.getString(1)
                    val content = c.getString(2)
                    val i = content.indexOf(q, ignoreCase = true)
                    if (i < 0 && !path.contains(q, ignoreCase = true)) continue
                    val snippet = if (i < 0) "" else
                        content.substring((i - 40).coerceAtLeast(0), (i + 80).coerceAtMost(content.length)).replace('\n', ' ')
                    add(SearchHit(c.getString(0), path, snippet))
                }
            }
        }.sortedBy { pathKey(it.path) }
    }

    fun pendingCount(): Int =
        readableDatabase.rawQuery(
            "SELECT (SELECT count(*) FROM notes WHERE dirty = 1) + (SELECT count(*) FROM files WHERE dirty = 1)", null,
        ).use { c -> if (c.moveToFirst()) c.getInt(0) else 0 }

    // ---------------------------------------------------------------- dossiers

    fun folders(): List<String> =
        readableDatabase.rawQuery("SELECT path FROM folders", null)
            .use { c -> buildList { while (c.moveToNext()) add(c.getString(0)) } }.sortedBy { pathKey(it) }

    fun replaceFolders(paths: Collection<String>): Unit = tx {
        val db = writableDatabase
        db.delete("folders", null, null)
        paths.forEach { db.insertWithOnConflict("folders", null, ContentValues().apply { put("path", it) }, SQLiteDatabase.CONFLICT_IGNORE) }
    }

    // ---------------------------------------------------------------- pièces jointes

    private fun file(c: android.database.Cursor) =
        FileRow(c.getString(0), c.getString(1), c.getLong(2), c.getString(3), c.getInt(4) != 0, c.getInt(5) != 0)

    fun files(withDeleted: Boolean = false): List<FileRow> =
        readableDatabase.rawQuery(
            "SELECT path, hash, size, local_hash, dirty, deleted FROM files ${if (withDeleted) "" else "WHERE deleted = 0"}", null,
        ).use { c -> buildList { while (c.moveToNext()) add(file(c)) } }.sortedBy { pathKey(it.path) }

    fun fileRow(path: String): FileRow? =
        readableDatabase.rawQuery("SELECT path, hash, size, local_hash, dirty, deleted FROM files WHERE path = ?", arrayOf(path))
            .use { c -> if (c.moveToFirst()) file(c) else null }

    fun putFile(row: FileRow) {
        writableDatabase.insertWithOnConflict(
            "files", null,
            ContentValues().apply {
                put("path", row.path); put("hash", row.hash); put("size", row.size); put("local_hash", row.localHash)
                put("dirty", if (row.dirty) 1 else 0); put("deleted", if (row.deleted) 1 else 0)
            },
            SQLiteDatabase.CONFLICT_REPLACE,
        )
    }

    /** Fichier du coffre. Un chemin qui en sortirait (`..`, absolu…) est refusé : il vient peut-être de la base. */
    fun fileOnDisk(path: String): File {
        require(isSafeRelPath(path)) { "Chemin de fichier refusé : $path" }
        return File(vaultDir, path).also { require(it.canonicalPath.startsWith(vaultRoot)) { "Chemin hors du coffre : $path" } }
    }

    /** Le fichier s'il est sur l'appareil, sinon null (pas téléchargé, ou chemin refusé). */
    fun localFile(path: String): File? = if (isSafeRelPath(path)) fileOnDisk(path).takeIf { it.isFile } else null

    private fun writeAtomic(f: File, bytes: ByteArray) {
        f.parentFile?.mkdirs()
        val tmp = File(f.parentFile, ".${f.name}.tmp")
        tmp.writeBytes(bytes)
        if (!tmp.renameTo(f)) {
            tmp.delete()
            throw java.io.IOException("Écriture impossible : ${f.name}")
        }
    }

    /**
     * Enregistre une pièce jointe faite sur l'appareil (whiteboard modifié) : à envoyer. [baseHash] est l'empreinte
     * du fichier que l'écran avait chargé : si le fichier a changé depuis (reçu d'un autre appareil), il n'est pas
     * écrasé et l'enregistrement part dans une copie « (conflit mobile) ». Renvoie le chemin écrit.
     */
    @Synchronized
    fun writeLocalFile(path: String, bytes: ByteArray, baseHash: String? = null): String = tx {
        val h = fileHash(bytes)
        val prev = fileRow(path)
        var target = path
        var known = prev?.hash
        if (baseHash != null && prev != null && prev.localHash != baseHash && prev.localHash != h) {
            val (base, ext) = fileConflictBase(path)
            val taken = files(withDeleted = true).mapTo(HashSet()) { pathKey(it.path) }
            target = firstFree(base, ext) { pathKey(it) in taken }
            known = null
        }
        writeAtomic(fileOnDisk(target), bytes)
        putFile(FileRow(target, known, bytes.size.toLong(), h, dirty = true, deleted = false))
        target
    }

    /**
     * Après l'envoi du fichier d'empreinte [sent] : la base a maintenant [hash]. « À jour » seulement si le fichier
     * n'a pas été réenregistré pendant l'envoi.
     */
    fun settleFile(path: String, sent: String, hash: String) {
        update(
            "UPDATE files SET hash = ?, dirty = CASE WHEN local_hash = ? AND deleted = 0 THEN 0 ELSE dirty END WHERE path = ?",
            hash, sent, path,
        )
    }

    /** Fichier d'empreinte [sent] mis de côté dans la base (conflit) ou non envoyable : la version de la base sera reprise. */
    fun releaseFile(path: String, sent: String, redownload: Boolean) {
        update(
            "UPDATE files SET dirty = 0, local_hash = CASE WHEN ? THEN NULL ELSE local_hash END WHERE path = ? AND local_hash = ? AND deleted = 0",
            redownload, path, sent,
        )
    }

    /**
     * Version de la base ([tmp] : contenu téléchargé, ou null si seule la ligne change). Ignorée si le fichier a été
     * modifié sur l'appareil entre-temps. Le fichier est remplacé d'un coup (jamais à moitié écrit).
     */
    @Synchronized
    fun installRemote(path: String, hash: String, size: Long, tmp: File?): Boolean = tx {
        val row = fileRow(path)
        if (row?.dirty == true) {
            tmp?.delete()
            return@tx false
        }
        var localHash = row?.localHash
        if (tmp != null) {
            val f = fileOnDisk(path)
            f.parentFile?.mkdirs()
            if (!tmp.renameTo(f)) {
                tmp.delete()
                return@tx false
            }
            localHash = hash
        }
        putFile(FileRow(path, hash, size, localHash, dirty = false, deleted = false))
        true
    }

    /** Fichier supprimé ailleurs : retiré, sauf s'il a été modifié ici entre-temps. */
    @Synchronized
    fun removeFileIfClean(path: String): Boolean = tx {
        (update("DELETE FROM files WHERE path = ? AND dirty = 0", path) > 0).also { if (it) localFile(path)?.delete() }
    }

    /** Suppression envoyée. */
    @Synchronized
    fun removeDeletedFile(path: String): Unit = tx {
        if (update("DELETE FROM files WHERE path = ? AND deleted = 1", path) > 0) localFile(path)?.delete()
    }

    // ---------------------------------------------------------------- divers

    fun meta(key: String): String? =
        readableDatabase.rawQuery("SELECT value FROM meta WHERE key = ?", arrayOf(key)).use { c ->
            if (c.moveToFirst()) c.getString(0) else null
        }

    fun setMeta(key: String, value: String?) {
        if (value == null) writableDatabase.delete("meta", "key = ?", arrayOf(key))
        else writableDatabase.insertWithOnConflict("meta", null, ContentValues().apply { put("key", key); put("value", value) }, SQLiteDatabase.CONFLICT_REPLACE)
    }

    /**
     * Changement de base : ce qui n'était que la copie de l'ancienne base est oublié (ces notes y sont toujours),
     * mais tout ce qui n'avait pas encore été envoyé est gardé et partira dans la nouvelle base comme du neuf.
     * Renvoie le nombre de notes et fichiers gardés.
     */
    @Synchronized
    fun forgetRemote(): Int = tx {
        update("DELETE FROM notes WHERE dirty = 0 OR deleted = 1")
        val notes = update("UPDATE notes SET version = 0")
        var kept = 0
        for (f in files(withDeleted = true)) {
            if (f.dirty && !f.deleted && localFile(f.path) != null) {
                putFile(f.copy(hash = null))
                kept++
            } else {
                update("DELETE FROM files WHERE path = ?", f.path)
                localFile(f.path)?.delete()
            }
        }
        writableDatabase.delete("folders", null, null)
        writableDatabase.delete("meta", "key IN ('board', 'last_sync')", null)
        notes + kept
    }
}
