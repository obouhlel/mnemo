package dev.mnemo.app.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.Article
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.CloudDone
import androidx.compose.material.icons.outlined.CloudOff
import androidx.compose.material.icons.outlined.Dashboard
import androidx.compose.material.icons.outlined.Draw
import androidx.compose.material.icons.outlined.Folder
import androidx.compose.material.icons.outlined.FolderOpen
import androidx.compose.material.icons.outlined.Image
import androidx.compose.material.icons.automirrored.outlined.InsertDriveFile
import androidx.compose.material.icons.outlined.PhoneAndroid
import androidx.compose.material.icons.outlined.Search
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material.icons.outlined.Sync
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.mnemo.app.data.SearchHit
import dev.mnemo.app.data.basename
import dev.mnemo.app.data.parentOf
import dev.mnemo.app.markdown.Preprocess
import kotlinx.coroutines.delay

/** Un élément de l'arborescence : dossier, note ou pièce jointe. */
private sealed class Entry(val path: String, val depth: Int) {
    class Folder(path: String, depth: Int, val open: Boolean) : Entry(path, depth)
    class Note(path: String, depth: Int, val id: String, val dirty: Boolean) : Entry(path, depth)
    class File(path: String, depth: Int) : Entry(path, depth)
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun NotesListScreen(model: AppModel) {
    val notes by model.notes.collectAsState()
    val folders by model.folders.collectAsState()
    val files by model.files.collectAsState()
    val sync by model.sync.collectAsState()
    val pending by model.pending.collectAsState()
    val screen by model.screen.collectAsState()
    val expanded = remember { mutableStateMapOf<String, Boolean>() }
    var searching by rememberSaveable { mutableStateOf(false) }
    var query by rememberSaveable { mutableStateOf("") }
    var menu by remember { mutableStateOf(false) }
    var newNote by remember { mutableStateOf(false) }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Mnemo") },
                colors = TopAppBarDefaults.topAppBarColors(containerColor = MaterialTheme.colorScheme.surface),
                actions = {
                    SyncIndicator(sync, pending) { model.syncNow() }
                    IconButton(onClick = { searching = !searching; if (!searching) query = "" }) {
                        Icon(if (searching) Icons.Outlined.Close else Icons.Outlined.Search, "Rechercher")
                    }
                    IconButton(onClick = { model.open(Screen.Dashboard) }) { Icon(Icons.Outlined.Dashboard, "Tableau de bord") }
                    IconButton(onClick = { model.open(Screen.Settings) }) { Icon(Icons.Outlined.Settings, "Paramètres") }
                },
            )
        },
        floatingActionButton = {
            Box {
                FloatingActionButton(onClick = { menu = true }, containerColor = MaterialTheme.colorScheme.primary) {
                    Icon(Icons.Outlined.Add, "Créer")
                }
                DropdownMenu(menu, onDismissRequest = { menu = false }) {
                    DropdownMenuItem(text = { Text("Nouvelle note") }, onClick = { menu = false; newNote = true })
                    DropdownMenuItem(text = { Text("Nouveau whiteboard") }, onClick = {
                        menu = false
                        model.open(Screen.Board(model.newBoard(null)))
                    })
                }
            }
        },
        containerColor = MaterialTheme.colorScheme.surface,
    ) { pad ->
        Column(Modifier.padding(pad).fillMaxSize()) {
            if (searching) {
                OutlinedTextField(
                    value = query,
                    onValueChange = { query = it },
                    placeholder = { Text("Rechercher dans les notes…") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 4.dp),
                )
            }
            if (query.isNotBlank()) {
                // recherche hors du fil principal, un instant après la dernière frappe (null : en cours)
                var found by remember { mutableStateOf<List<SearchHit>?>(null) }
                LaunchedEffect(query, notes) {
                    delay(250)
                    found = model.search(query)
                }
                val hits = found.orEmpty()
                LazyColumn(Modifier.fillMaxSize()) {
                    items(hits, key = { it.id }) { n ->
                        val snippet = n.snippet
                        Column(Modifier.fillMaxWidth().clickable { model.open(Screen.Note(n.id)) }.padding(horizontal = 16.dp, vertical = 10.dp)) {
                            Text(basename(n.path), style = MaterialTheme.typography.bodyLarge)
                            if (snippet.isNotEmpty()) Text(snippet, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 2)
                        }
                    }
                    if (found != null && hits.isEmpty()) item { Text("Aucun résultat.", Modifier.padding(16.dp), color = MaterialTheme.colorScheme.onSurfaceVariant) }
                }
            } else {
                val open = expanded.toMap()
                val entries = remember(notes, folders, files, open) {
                    buildTree(notes.map { Triple(it.path, it.id, it.dirty) }, folders, files.map { it.path }, open)
                }
                LazyColumn(Modifier.fillMaxSize(), contentPadding = androidx.compose.foundation.layout.PaddingValues(bottom = 88.dp)) {
                    items(entries, key = {
                        // une note est désignée par son identifiant : deux notes peuvent avoir le même chemin (avant synchronisation)
                        when (it) {
                            is Entry.Note -> "n:" + it.id
                            is Entry.Folder -> "d:" + it.path
                            is Entry.File -> "f:" + it.path
                        }
                    }) { e ->
                        val selected = when (e) {
                            is Entry.Note -> screen == Screen.Note(e.id)
                            is Entry.File -> screen == Screen.Board(e.path) || screen == Screen.Image(e.path)
                            else -> false
                        }
                        TreeRow(e, selected) {
                            when (e) {
                                is Entry.Folder -> expanded[e.path.lowercase()] = !e.open
                                is Entry.Note -> model.open(Screen.Note(e.id))
                                is Entry.File -> when {
                                    e.path.endsWith(".mnemo-board", true) -> model.open(Screen.Board(e.path))
                                    Preprocess.isImage(e.path) -> model.open(Screen.Image(e.path))
                                    else -> model.say("Ce type de fichier ne s'ouvre pas encore dans l'appli")
                                }
                            }
                        }
                    }
                    if (entries.isEmpty()) item {
                        Text("Aucune note pour l'instant.", Modifier.padding(16.dp), color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
            }
        }
    }
    if (newNote) {
        var path by remember { mutableStateOf("") }
        val error = remember(path) { if (path.isBlank()) null else model.pathError(path) }
        AlertDialog(
            onDismissRequest = { newNote = false },
            title = { Text("Nouvelle note") },
            text = {
                OutlinedTextField(
                    path, { path = it }, label = { Text("Dossier/Nom") }, singleLine = true,
                    isError = error != null, supportingText = error?.let { { Text(it) } },
                )
            },
            confirmButton = {
                TextButton(enabled = path.isNotBlank() && error == null, onClick = {
                    model.createNote(path)?.let { model.open(Screen.Note(it)) }
                    newNote = false
                }) { Text("Créer") }
            },
            dismissButton = { TextButton(onClick = { newNote = false }) { Text("Annuler") } },
        )
    }
}

@Composable
private fun SyncIndicator(sync: SyncState, pending: Int, onClick: () -> Unit) {
    val (icon, tint, label) = when (sync) {
        SyncState.LocalOnly -> Triple(Icons.Outlined.PhoneAndroid, MaterialTheme.colorScheme.onSurfaceVariant, "Sur cet appareil uniquement")
        SyncState.Syncing -> Triple(Icons.Outlined.Sync, MaterialTheme.colorScheme.onSurfaceVariant, "Synchronisation…")
        is SyncState.Synced -> Triple(Icons.Outlined.CloudDone, androidx.compose.ui.graphics.Color(0xFF44CF6E), "Synchronisé")
        is SyncState.Offline -> Triple(Icons.Outlined.CloudOff, androidx.compose.ui.graphics.Color(0xFFE9973F), "Hors ligne")
    }
    IconButton(onClick = onClick) {
        Box {
            Icon(icon, label, tint = tint)
            if (pending > 0) {
                Text(
                    pending.toString(),
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.primary,
                    modifier = Modifier.align(Alignment.BottomEnd),
                )
            }
        }
    }
}

@Composable
private fun TreeRow(e: Entry, selected: Boolean, onClick: () -> Unit) {
    val icon: ImageVector = when (e) {
        is Entry.Folder -> if (e.open) Icons.Outlined.FolderOpen else Icons.Outlined.Folder
        is Entry.Note -> Icons.AutoMirrored.Outlined.Article
        is Entry.File -> when {
            e.path.endsWith(".mnemo-board", true) -> Icons.Outlined.Draw
            Preprocess.isImage(e.path) -> Icons.Outlined.Image
            else -> Icons.AutoMirrored.Outlined.InsertDriveFile
        }
    }
    val bg = if (selected) MaterialTheme.colorScheme.surfaceVariant else androidx.compose.ui.graphics.Color.Transparent
    Row(
        Modifier.fillMaxWidth().padding(horizontal = 8.dp).height(44.dp)
            .background(bg, MaterialTheme.shapes.small).clickable(onClick = onClick)
            .padding(start = (8 + e.depth * 18).dp, end = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.Start,
    ) {
        Icon(icon, null, Modifier.size(20.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
        Spacer(Modifier.width(12.dp))
        Text(
            basename(e.path) + if (e is Entry.Note && e.dirty) "  •" else "",
            style = MaterialTheme.typography.bodyLarge,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

/** Arborescence aplatie (dossiers dépliés seulement), dossiers d'abord, puis notes, puis pièces jointes. */
private fun buildTree(
    notes: List<Triple<String, String, Boolean>>,
    folders: List<String>,
    files: List<String>,
    expanded: Map<String, Boolean>,
): List<Entry> {
    val dirs = sortedSetOf(String.CASE_INSENSITIVE_ORDER)
    fun addParents(p: String) {
        var cur = parentOf(p)
        while (cur != null) {
            dirs += cur
            cur = parentOf(cur)
        }
    }
    folders.forEach { dirs += it; addParents(it) }
    notes.forEach { addParents(it.first) }
    files.forEach { addParents(it) }
    val out = mutableListOf<Entry>()
    fun visit(dir: String?, depth: Int) {
        // sans casse : « École/a » et « école/b » sont dans le même dossier
        val inDir = { p: String -> parentOf(p)?.lowercase() == dir?.lowercase() }
        dirs.filter(inDir).forEach { d ->
            val open = expanded[d.lowercase()] == true
            out += Entry.Folder(d, depth, open)
            if (open) visit(d, depth + 1)
        }
        notes.filter { inDir(it.first) }.sortedBy { it.first.lowercase() }.forEach { out += Entry.Note(it.first, depth, it.second, it.third) }
        files.filter(inDir).sortedBy { it.lowercase() }.forEach { out += Entry.File(it, depth) }
    }
    visit(null, 0)
    return out
}
