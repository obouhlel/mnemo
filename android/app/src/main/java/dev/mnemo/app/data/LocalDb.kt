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
 */
class LocalDb(context: Context, name: String = "mnemo.db") :
    SQLiteOpenHelper(context, name, null, 1) {

    val vaultDir: File = File(context.filesDir, "vault").apply { mkdirs() }

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

    // ---------------------------------------------------------------- notes

    private fun note(c: android.database.Cursor) = NoteRow(
        c.getString(0), c.getString(1), c.getString(2), c.getInt(3), c.getLong(4), c.getInt(5) != 0, c.getInt(6) != 0,
    )

    private val noteCols = "id, path, content, version, updated_at, dirty, deleted"

    /** Toutes les notes, supprimées comprises si [withDeleted]. */
    fun notes(withDeleted: Boolean = false): List<NoteRow> =
        readableDatabase.rawQuery(
            "SELECT $noteCols FROM notes ${if (withDeleted) "" else "WHERE deleted = 0"} ORDER BY lower(path)", null,
        ).use { c -> buildList { while (c.moveToNext()) add(note(c)) } }

    fun note(id: String): NoteRow? =
        readableDatabase.rawQuery("SELECT $noteCols FROM notes WHERE id = ?", arrayOf(id)).use { c ->
            if (c.moveToFirst()) note(c) else null
        }

    fun noteByPath(path: String): NoteRow? =
        readableDatabase.rawQuery("SELECT $noteCols FROM notes WHERE lower(path) = lower(?) AND deleted = 0", arrayOf(path))
            .use { c -> if (c.moveToFirst()) note(c) else null }

    /** Note reçue de la base (remplace la copie locale, qui n'est pas modifiée). */
    fun putRemote(id: String, path: String, content: String, version: Int, updatedAt: Long) {
        writableDatabase.insertWithOnConflict(
            "notes", null,
            ContentValues().apply {
                put("id", id); put("path", path); put("content", content); put("version", version)
                put("updated_at", updatedAt); put("dirty", 0); put("deleted", 0)
            },
            SQLiteDatabase.CONFLICT_REPLACE,
        )
    }

    /** Modification faite sur l'appareil : à envoyer. */
    fun saveLocal(id: String, path: String, content: String) {
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

    fun markDeleted(id: String) {
        writableDatabase.execSQL("UPDATE notes SET deleted = 1, dirty = 1 WHERE id = ?", arrayOf(id))
    }

    fun markClean(id: String, version: Int) {
        writableDatabase.execSQL("UPDATE notes SET dirty = 0, version = ? WHERE id = ?", arrayOf(version, id))
    }

    fun removeNote(id: String) {
        writableDatabase.delete("notes", "id = ?", arrayOf(id))
    }

    fun pendingCount(): Int =
        readableDatabase.rawQuery(
            "SELECT (SELECT count(*) FROM notes WHERE dirty = 1) + (SELECT count(*) FROM files WHERE dirty = 1)", null,
        ).use { c -> if (c.moveToFirst()) c.getInt(0) else 0 }

    // ---------------------------------------------------------------- dossiers

    fun folders(): List<String> =
        readableDatabase.rawQuery("SELECT path FROM folders ORDER BY lower(path)", null)
            .use { c -> buildList { while (c.moveToNext()) add(c.getString(0)) } }

    fun replaceFolders(paths: Collection<String>) {
        val db = writableDatabase
        db.beginTransaction()
        try {
            db.delete("folders", null, null)
            paths.forEach { db.insertWithOnConflict("folders", null, ContentValues().apply { put("path", it) }, SQLiteDatabase.CONFLICT_IGNORE) }
            db.setTransactionSuccessful()
        } finally {
            db.endTransaction()
        }
    }

    // ---------------------------------------------------------------- pièces jointes

    private fun file(c: android.database.Cursor) =
        FileRow(c.getString(0), c.getString(1), c.getLong(2), c.getString(3), c.getInt(4) != 0, c.getInt(5) != 0)

    fun files(withDeleted: Boolean = false): List<FileRow> =
        readableDatabase.rawQuery(
            "SELECT path, hash, size, local_hash, dirty, deleted FROM files ${if (withDeleted) "" else "WHERE deleted = 0"} ORDER BY lower(path)",
            null,
        ).use { c -> buildList { while (c.moveToNext()) add(file(c)) } }

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

    fun removeFile(path: String) {
        writableDatabase.delete("files", "path = ?", arrayOf(path))
        File(vaultDir, path).delete()
    }

    fun fileOnDisk(path: String): File = File(vaultDir, path)

    /** Enregistre une pièce jointe faite sur l'appareil (photo, whiteboard modifié) : à envoyer. */
    fun writeLocalFile(path: String, bytes: ByteArray) {
        val f = fileOnDisk(path)
        f.parentFile?.mkdirs()
        val tmp = File(f.parentFile, ".${f.name}.tmp")
        tmp.writeBytes(bytes)
        tmp.renameTo(f)
        val prev = fileRow(path)
        val h = fileHash(bytes)
        putFile(FileRow(path, prev?.hash, bytes.size.toLong(), h, dirty = true, deleted = false))
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

    /** Efface toute la copie locale (changement de base). */
    fun reset() {
        val db = writableDatabase
        listOf("notes", "files", "folders").forEach { db.delete(it, null, null) }
        db.delete("meta", "key <> 'device_id'", null)
        vaultDir.deleteRecursively()
        vaultDir.mkdirs()
    }
}
