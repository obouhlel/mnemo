package dev.mnemo.app.ui

import android.app.Application
import android.content.Context
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import dev.mnemo.app.data.DbConfig
import dev.mnemo.app.data.FileRow
import dev.mnemo.app.data.LocalDb
import dev.mnemo.app.data.NoteRow
import dev.mnemo.app.data.SyncEngine
import dev.mnemo.app.data.basename
import dev.mnemo.app.data.parseDbUrl
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import java.util.UUID

/** État de la synchronisation, affiché en haut de la liste. */
sealed class SyncState {
    data object LocalOnly : SyncState()
    data object Syncing : SyncState()
    data class Synced(val at: Long) : SyncState()
    data class Offline(val error: String) : SyncState()
}

/** Ce qui est ouvert à droite (tablette) ou en plein écran (téléphone). */
sealed class Screen {
    data object List : Screen()
    data class Note(val id: String) : Screen()
    data class Board(val path: String) : Screen()
    data class Image(val path: String) : Screen()
    data object Dashboard : Screen()
    data object Settings : Screen()
}

// @JvmOverloads : Android crée le ViewModel par réflexion avec le seul constructeur (Application)
class AppModel @JvmOverloads constructor(app: Application, dbName: String = "mnemo.db") : AndroidViewModel(app) {
    val local = LocalDb(app, dbName)
    private val prefs = app.getSharedPreferences("mnemo", Context.MODE_PRIVATE)
    /** adresse de la base (vide : notes seulement sur l'appareil), observable par les écrans */
    private val _url = MutableStateFlow(prefs.getString("db_url", "") ?: "")
    val url: StateFlow<String> = _url.asStateFlow()

    private val engine = SyncEngine(local, clientId())
    private val syncLock = Mutex()

    private val _notes = MutableStateFlow<List<NoteRow>>(emptyList())
    val notes: StateFlow<List<NoteRow>> = _notes.asStateFlow()
    private val _folders = MutableStateFlow<List<String>>(emptyList())
    val folders: StateFlow<List<String>> = _folders.asStateFlow()
    private val _files = MutableStateFlow<List<FileRow>>(emptyList())
    val files: StateFlow<List<FileRow>> = _files.asStateFlow()
    private val _sync = MutableStateFlow<SyncState>(SyncState.LocalOnly)
    val sync: StateFlow<SyncState> = _sync.asStateFlow()
    private val _pending = MutableStateFlow(0)
    val pending: StateFlow<Int> = _pending.asStateFlow()
    private val _screen = MutableStateFlow<Screen>(Screen.List)
    val screen: StateFlow<Screen> = _screen.asStateFlow()
    private val _message = MutableStateFlow<String?>(null)
    val message: StateFlow<String?> = _message.asStateFlow()
    /** incrémenté à chaque synchronisation qui a changé quelque chose (les écrans ouverts se rechargent) */
    private val _revision = MutableStateFlow(0)
    val revision: StateFlow<Int> = _revision.asStateFlow()

    private var loop: Job? = null
    private var debounce: Job? = null

    init {
        reload()
        if (dbUrl().isNotBlank()) _sync.value = SyncState.Offline("Pas encore synchronisé")
    }

    private fun clientId(): String {
        val existing = local.meta("device_id")
        if (existing != null) return existing
        val id = "android-" + UUID.randomUUID().toString()
        local.setMeta("device_id", id)
        return id
    }

    fun dbUrl(): String = _url.value

    fun config(): DbConfig? = parseDbUrl(dbUrl())

    fun reload() {
        _notes.value = local.notes()
        _folders.value = local.folders()
        _files.value = local.files()
        _pending.value = local.pendingCount()
    }

    fun open(s: Screen) {
        _screen.value = s
    }

    fun clearMessage() {
        _message.value = null
    }

    fun say(m: String) {
        _message.value = m
    }

    // ---------------------------------------------------------------- synchronisation

    /** Synchronise maintenant (une seule à la fois). */
    fun syncNow(quiet: Boolean = false) {
        val cfg = config() ?: return
        viewModelScope.launch {
            syncLock.withLock {
                _sync.value = SyncState.Syncing
                val result = withContext(Dispatchers.IO) { runCatching { engine.sync(cfg) } }
                result.onSuccess { r ->
                    _sync.value = SyncState.Synced(System.currentTimeMillis())
                    if (r.pulled + r.pushed + r.files + r.conflicts > 0) _revision.value++
                    if (r.conflicts > 0) say("${r.conflicts} conflit(s) : vos modifications sont gardées dans « … (conflit mobile) »")
                }.onFailure { e ->
                    android.util.Log.w("Mnemo", "synchronisation", e)
                    val msg = engine.readable(e)
                    _sync.value = SyncState.Offline(msg)
                    if (!quiet) say("Synchronisation impossible : $msg")
                }
                reload()
            }
        }
    }

    /** Synchronisation périodique tant que l'appli est au premier plan. */
    fun startLoop() {
        if (loop?.isActive == true || config() == null) return
        loop = viewModelScope.launch {
            while (isActive) {
                syncNow(quiet = true)
                delay(15_000)
            }
        }
    }

    fun stopLoop() {
        loop?.cancel()
        loop = null
    }

    private fun scheduleSync() {
        reload()
        if (config() == null) return
        debounce?.cancel()
        debounce = viewModelScope.launch {
            delay(1500)
            syncNow(quiet = true)
        }
    }

    /** Teste puis enregistre l'adresse de la base ; vide la copie locale si la base change. */
    fun connect(url: String, done: (String?) -> Unit) {
        val cfg = parseDbUrl(url)
        if (cfg == null) {
            done("Adresse invalide (attendu : postgres://utilisateur:motdepasse@hôte:5432/base)")
            return
        }
        viewModelScope.launch {
            val err = withContext(Dispatchers.IO) { engine.test(cfg) }
            android.util.Log.i("Mnemo", "connexion : ${err ?: "ok"}")
            if (err == null) {
                if (url.trim() != dbUrl()) withContext(Dispatchers.IO) { local.reset() }
                prefs.edit().putString("db_url", url.trim()).apply()
                _url.value = url.trim()
                reload()
                stopLoop()
                startLoop()
            }
            done(err)
        }
    }

    fun disconnect() {
        stopLoop()
        prefs.edit().remove("db_url").apply()
        _url.value = ""
        _sync.value = SyncState.LocalOnly
    }

    // ---------------------------------------------------------------- notes

    fun note(id: String): NoteRow? = local.note(id)

    fun saveNote(id: String, path: String, content: String) {
        local.saveLocal(id, path, content)
        scheduleSync()
    }

    fun createNote(rawPath: String): String? {
        val path = rawPath.trim().removeSuffix(".md").split('/').map { it.trim() }.filter { it.isNotEmpty() }.joinToString("/")
        if (path.isEmpty()) return null
        local.noteByPath(path)?.let { return it.id }
        val id = UUID.randomUUID().toString()
        local.saveLocal(id, path, "")
        scheduleSync()
        return id
    }

    fun renameNote(id: String, newPath: String) {
        val n = local.note(id) ?: return
        val path = newPath.trim().removeSuffix(".md")
        if (path.isEmpty() || path == n.path) return
        local.saveLocal(id, path, n.content)
        scheduleSync()
    }

    fun deleteNote(id: String) {
        local.markDeleted(id)
        if (_screen.value == Screen.Note(id)) _screen.value = Screen.List
        scheduleSync()
    }

    /** Note désignée par un lien `[[…]]` : chemin exact, sinon nom (le plus court). */
    fun resolveLink(target: String): NoteRow? {
        val t = target.trim().removeSuffix(".md").lowercase()
        return _notes.value.filter { it.path.lowercase() == t || basename(it.path).lowercase() == t }
            .minWithOrNull(compareBy({ it.path.lowercase() != t }, { it.path.length }))
    }

    // ---------------------------------------------------------------- pièces jointes / whiteboards

    /** Pièce jointe désignée par `![[…]]` depuis une note (dossier de la note, chemin complet ou nom seul). */
    fun findFile(target: String, notePath: String): FileRow? {
        val t = target.trim().removePrefix("./").lowercase()
        val dir = dev.mnemo.app.data.parentOf(notePath)
        val files = _files.value
        if (dir != null) files.firstOrNull { it.path.lowercase() == "${dir.lowercase()}/$t" }?.let { return it }
        return files.filter { it.path.lowercase() == t || basename(it.path).lowercase() == t }.minByOrNull { it.path.length }
    }

    fun readFile(path: String): ByteArray? = local.fileOnDisk(path).takeIf { it.exists() }?.readBytes()

    fun writeFile(path: String, bytes: ByteArray) {
        local.writeLocalFile(path, bytes)
        scheduleSync()
    }

    fun newBoard(folder: String?): String {
        val name = "Whiteboard " + java.text.SimpleDateFormat("yyyy-MM-dd HHmm", java.util.Locale.FRANCE).format(java.util.Date())
        var path = listOfNotNull(folder?.takeIf { it.isNotEmpty() }, "$name.mnemo-board").joinToString("/")
        var n = 1
        while (_files.value.any { it.path.equals(path, true) }) {
            path = listOfNotNull(folder?.takeIf { it.isNotEmpty() }, "$name $n.mnemo-board").joinToString("/")
            n++
        }
        writeFile(path, dev.mnemo.app.data.Sketch().toJson().toByteArray())
        return path
    }

    fun boardJson(): String? = local.meta("board")
}
