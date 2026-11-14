package dev.mnemo.app.ui

import android.app.Application
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import dev.mnemo.app.data.DbConfig
import dev.mnemo.app.data.DbIdentity
import dev.mnemo.app.data.FileRow
import dev.mnemo.app.data.LocalDb
import dev.mnemo.app.data.NoteMeta
import dev.mnemo.app.data.NoteRow
import dev.mnemo.app.data.SearchHit
import dev.mnemo.app.data.SecretStore
import dev.mnemo.app.data.SyncEngine
import dev.mnemo.app.data.basename
import dev.mnemo.app.data.newNotePath
import dev.mnemo.app.data.parseDbUrl
import dev.mnemo.app.data.pathKey
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

/** Image d'une pièce jointe, décodée hors du fil principal. */
sealed class ImageLoad {
    data object Loading : ImageLoad()
    data object Missing : ImageLoad()
    data object Unreadable : ImageLoad()
    data class Ready(val bitmap: Bitmap) : ImageLoad()
}

/** Réduction (puissance de 2) pour qu'une image de [w] × [h] tienne dans [max] pixels de côté. */
fun sampleSize(w: Int, h: Int, max: Int): Int {
    var s = 1
    while (w / s > max || h / s > max) s *= 2
    return s
}

// @JvmOverloads : Android crée le ViewModel par réflexion avec le seul constructeur (Application)
class AppModel @JvmOverloads constructor(app: Application, dbName: String = "mnemo.db") : AndroidViewModel(app) {
    val local = LocalDb(app, dbName)
    private val secrets = SecretStore(app)
    /** adresse de la base (vide : notes seulement sur l'appareil), observable par les écrans */
    private val _url = MutableStateFlow(secrets.dbUrl())
    val url: StateFlow<String> = _url.asStateFlow()

    private val engine = SyncEngine(local, clientId())
    private val syncLock = Mutex()
    // fil principal uniquement : une synchronisation à la fois, et une de plus si on en a demandé une pendant
    private var syncing = false
    private var syncAgain = false

    private val _notes = MutableStateFlow<List<NoteMeta>>(emptyList())
    val notes: StateFlow<List<NoteMeta>> = _notes.asStateFlow()
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

    /** Recharge les listes (sans le texte des notes : rien de lourd sur le fil principal). */
    fun reload() {
        _notes.value = local.noteList()
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

    /** Synchronise maintenant. Une seule à la fois : demandée pendant qu'une autre tourne, elle la suit. */
    fun syncNow(quiet: Boolean = false) {
        viewModelScope.launch { syncOnce(quiet) }
    }

    private suspend fun syncOnce(quiet: Boolean) {
        if (syncing) {
            syncAgain = true
            return
        }
        syncing = true
        try {
            do {
                syncAgain = false
                val cfg = config() ?: return
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
            } while (syncAgain)
        } finally {
            syncing = false
        }
    }

    /** Synchronisation périodique tant que l'appli est au premier plan : 15 s après la fin de la précédente. */
    fun startLoop() {
        if (loop?.isActive == true || config() == null) return
        loop = viewModelScope.launch {
            while (isActive) {
                syncOnce(quiet = true)
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
            syncOnce(quiet = true)
        }
    }

    /** Base avec laquelle la copie locale a été synchronisée (null : aucune). */
    private fun knownDb(): DbIdentity? {
        val addr = local.meta("db_addr") ?: config()?.identity ?: return null
        return DbIdentity(addr, local.meta("db_id"))
    }

    /**
     * Teste puis enregistre l'adresse de la base. Les notes de l'appareil ne sont jamais effacées pour autant :
     * même base sous une autre adresse (mot de passe, `?sslmode=…`), rien ne change ; autre base, la copie de
     * l'ancienne est oubliée mais tout ce qui n'avait pas été envoyé est gardé et part dans la nouvelle.
     */
    fun connect(url: String, done: (String?) -> Unit) {
        val cfg = parseDbUrl(url)
        if (cfg == null) {
            done("Adresse invalide (attendu : postgres://utilisateur:motdepasse@hôte:5432/base ; sslmode : disable, require, verify-ca ou verify-full)")
            return
        }
        viewModelScope.launch {
            var identity: DbIdentity? = null
            val err = withContext(Dispatchers.IO) {
                engine.test(cfg) ?: runCatching { identity = engine.identity(cfg) }.exceptionOrNull()?.let { engine.readable(it) }
            }
            android.util.Log.i("Mnemo", "connexion : ${err ?: "ok"}")
            val db = identity
            if (err == null && db != null) {
                // sous le verrou : jamais pendant une synchronisation avec l'ancienne base
                val kept = syncLock.withLock {
                    val kept = withContext(Dispatchers.IO) {
                        val other = knownDb()?.sameAs(db) != true
                        (if (other) local.forgetRemote() else 0).also {
                            local.setMeta("db_addr", db.addr)
                            local.setMeta("db_id", db.id)
                        }
                    }
                    secrets.setDbUrl(url.trim())
                    _url.value = url.trim()
                    kept
                }
                if (kept > 0) say("$kept modification(s) pas encore envoyée(s) : gardées, elles partent dans cette base")
                reload()
                _revision.value++
                stopLoop()
                startLoop()
            }
            done(err)
        }
    }

    fun disconnect() {
        stopLoop()
        secrets.setDbUrl(null)
        _url.value = ""
        _sync.value = SyncState.LocalOnly
    }

    // ---------------------------------------------------------------- notes

    fun note(id: String): NoteRow? = local.note(id)

    /**
     * Enregistre le texte de l'éditeur ([base] : le texte qu'il avait chargé). Renvoie l'identifiant de la note qui
     * le contient : une autre note « (conflit mobile …) » si celle-ci a été modifiée ailleurs pendant la saisie.
     */
    fun saveNote(id: String, path: String, base: String?, content: String): String {
        val target = local.saveEdit(id, path, base, content)
        if (target != id) say("Note modifiée sur un autre appareil pendant la saisie : votre texte est gardé dans « … (conflit mobile) »")
        scheduleSync()
        return target
    }

    /** Message d'erreur si [raw] ne peut pas être le chemin d'une note ; [renaming] : la note renommée (nom déjà pris refusé). */
    fun pathError(raw: String, renaming: String? = null): String? {
        val path = newNotePath(raw).getOrElse { return it.message }
        val key = pathKey(path)
        return if (renaming != null && local.noteList().any { it.id != renaming && pathKey(it.path) == key }) "Une note porte déjà ce nom" else null
    }

    /** Crée la note (ou renvoie celle qui existe déjà à ce chemin) ; null si le nom est refusé. */
    fun createNote(rawPath: String): String? {
        val path = newNotePath(rawPath).getOrElse {
            say(it.message ?: "Nom invalide")
            return null
        }
        local.noteByPath(path)?.let { return it.id }
        val id = UUID.randomUUID().toString()
        local.saveLocal(id, path, "")
        scheduleSync()
        return id
    }

    /** Renomme ; renvoie le nouveau chemin, ou null si le nom est refusé (voir [pathError]). */
    fun renameNote(id: String, newPath: String): String? {
        if (pathError(newPath, renaming = id) != null) return null
        val path = newNotePath(newPath).getOrNull() ?: return null
        local.rename(id, path)
        scheduleSync()
        return path
    }

    fun deleteNote(id: String) {
        local.markDeleted(id)
        if (_screen.value == Screen.Note(id)) _screen.value = Screen.List
        scheduleSync()
    }

    /** Note désignée par un lien `[[…]]` : chemin exact, sinon nom (le plus court). */
    fun resolveLink(target: String): NoteMeta? {
        val t = target.trim().removeSuffix(".md").lowercase()
        return _notes.value.filter { it.path.lowercase() == t || basename(it.path).lowercase() == t }
            .minWithOrNull(compareBy({ it.path.lowercase() != t }, { it.path.length }))
    }

    suspend fun search(query: String): List<SearchHit> = withContext(Dispatchers.IO) { local.search(query) }

    // ---------------------------------------------------------------- pièces jointes / whiteboards

    /** Pièce jointe désignée par `![[…]]` depuis une note (dossier de la note, chemin complet ou nom seul). */
    fun findFile(target: String, notePath: String): FileRow? {
        val t = target.trim().removePrefix("./").lowercase()
        val dir = dev.mnemo.app.data.parentOf(notePath)
        val files = _files.value
        if (dir != null) files.firstOrNull { it.path.lowercase() == "${dir.lowercase()}/$t" }?.let { return it }
        return files.filter { it.path.lowercase() == t || basename(it.path).lowercase() == t }.minByOrNull { it.path.length }
    }

    fun readFile(path: String): ByteArray? = local.localFile(path)?.readBytes()

    /** Le fichier est-il connu (même pas encore téléchargé) ? */
    fun knowsFile(path: String): Boolean = local.fileRow(path) != null

    /**
     * Image décodée hors du fil principal, réduite pour tenir dans [maxSide] pixels de côté
     * (une photo de 12 Mpx en pleine taille : ~48 Mo de mémoire).
     */
    suspend fun loadImage(path: String, maxSide: Int = 2048): ImageLoad = withContext(Dispatchers.IO) {
        val file = local.localFile(path) ?: return@withContext ImageLoad.Missing
        runCatching {
            val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
            BitmapFactory.decodeFile(file.path, bounds)
            if (bounds.outWidth <= 0 || bounds.outHeight <= 0) return@runCatching null
            BitmapFactory.decodeFile(file.path, BitmapFactory.Options().apply { inSampleSize = sampleSize(bounds.outWidth, bounds.outHeight, maxSide) })
        }.getOrNull()?.let { ImageLoad.Ready(it) } ?: ImageLoad.Unreadable
    }

    /**
     * Enregistre une pièce jointe ([baseHash] : empreinte du fichier que l'écran avait chargé). Renvoie le chemin
     * écrit : une copie « (conflit mobile) » si le fichier a été modifié ailleurs entre-temps.
     */
    fun writeFile(path: String, bytes: ByteArray, baseHash: String? = null): String {
        val target = local.writeLocalFile(path, bytes, baseHash)
        if (target != path) say("Modifié sur un autre appareil entre-temps : votre version est gardée dans « ${basename(target)} »")
        scheduleSync()
        return target
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
