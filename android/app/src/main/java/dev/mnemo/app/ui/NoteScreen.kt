package dev.mnemo.app.ui

import android.content.Intent
import android.net.Uri
import android.widget.TextView
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.outlined.Edit
import androidx.compose.material.icons.outlined.MenuBook
import androidx.compose.material.icons.outlined.MoreVert
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
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
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import dev.mnemo.app.data.basename
import dev.mnemo.app.markdown.Preprocess
import io.noties.markwon.AbstractMarkwonPlugin
import io.noties.markwon.Markwon
import io.noties.markwon.MarkwonConfiguration
import io.noties.markwon.ext.strikethrough.StrikethroughPlugin
import io.noties.markwon.ext.tables.TablePlugin
import io.noties.markwon.ext.tasklist.TaskListPlugin
import kotlinx.coroutines.delay

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun NoteScreen(model: AppModel, id: String, onBack: (() -> Unit)?) {
    val revision by model.revision.collectAsState()
    val files by model.files.collectAsState()
    val initial = remember(id) { model.note(id)?.takeIf { !it.deleted } }
    if (initial == null) {
        EmptyDetail("Cette note n'existe plus.")
        return
    }
    var path by remember(id) { mutableStateOf(initial.path) }
    var text by remember(id) { mutableStateOf(initial.content) }
    /** le texte de la note tel que l'éditeur le connaît dans la copie locale (chargé ou enregistré) */
    var saved by remember(id) { mutableStateOf(initial.content) }
    var editing by rememberSaveable(id) { mutableStateOf(initial.content.isEmpty()) }
    var menu by remember { mutableStateOf(false) }
    var renaming by remember { mutableStateOf(false) }
    var confirmDelete by remember { mutableStateOf(false) }

    // Enregistre la saisie en attente. Si la note a changé ailleurs entre-temps, la saisie part dans une note
    // « (conflit mobile …) » : avec `follow`, l'éditeur continue dans cette note.
    fun flush(follow: Boolean) {
        if (text == saved) return
        val target = model.saveNote(id, path, saved, text)
        saved = text
        if (target != id && follow) model.open(Screen.Note(target))
    }

    // modifiée sur un autre appareil : rechargée si rien n'est en cours ici
    LaunchedEffect(revision) {
        val fresh = model.note(id)?.takeIf { !it.deleted } ?: return@LaunchedEffect
        if (text == saved && fresh.content != saved) {
            text = fresh.content
            saved = fresh.content
        }
        path = fresh.path
    }
    // enregistrement automatique 0,8 s après la dernière frappe…
    LaunchedEffect(text) {
        if (text != saved) {
            delay(800)
            flush(follow = true)
        }
    }
    // … et sans attendre en quittant la note (retour, autre note, rotation) ou l'appli
    DisposableEffect(id) { onDispose { flush(follow = false) } }
    LifecycleEventEffect(Lifecycle.Event.ON_STOP) { flush(follow = true) }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(basename(path), maxLines = 1) },
                navigationIcon = {
                    if (onBack != null) IconButton(onClick = {
                        flush(follow = false)
                        onBack()
                    }) { Icon(Icons.AutoMirrored.Outlined.ArrowBack, "Retour") }
                },
                actions = {
                    IconButton(onClick = {
                        flush(follow = true)
                        editing = !editing
                    }) {
                        Icon(if (editing) Icons.Outlined.MenuBook else Icons.Outlined.Edit, if (editing) "Lecture" else "Modifier")
                    }
                    Box {
                        IconButton(onClick = { menu = true }) { Icon(Icons.Outlined.MoreVert, "Plus") }
                        DropdownMenu(menu, onDismissRequest = { menu = false }) {
                            DropdownMenuItem(text = { Text("Renommer / déplacer") }, onClick = { menu = false; renaming = true })
                            DropdownMenuItem(text = { Text("Supprimer", color = MaterialTheme.colorScheme.error) }, onClick = { menu = false; confirmDelete = true })
                        }
                    }
                },
                colors = TopAppBarDefaults.topAppBarColors(containerColor = MaterialTheme.colorScheme.background),
            )
        },
        containerColor = MaterialTheme.colorScheme.background,
    ) { pad ->
        // imePadding : le clavier réduit la zone de saisie au lieu de la recouvrir (bord à bord, Android 15)
        Box(Modifier.padding(pad).consumeWindowInsets(pad).imePadding().fillMaxSize(), contentAlignment = Alignment.TopCenter) {
            Column(Modifier.widthIn(max = 760.dp).fillMaxSize().padding(horizontal = 20.dp)) {
                if (editing) {
                    BasicTextField(
                        value = text,
                        onValueChange = { text = it },
                        textStyle = TextStyle(color = MaterialTheme.colorScheme.onBackground, fontSize = 17.sp, lineHeight = 26.sp),
                        cursorBrush = SolidColor(MaterialTheme.colorScheme.primary),
                        modifier = Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(vertical = 12.dp),
                    )
                } else {
                    Rendered(model, path, text, files)
                }
            }
        }
    }

    if (renaming) {
        var p by remember { mutableStateOf(path) }
        val error = remember(p) { model.pathError(p, renaming = id) }
        AlertDialog(
            onDismissRequest = { renaming = false },
            title = { Text("Renommer / déplacer") },
            text = {
                OutlinedTextField(
                    p, { p = it }, label = { Text("Dossier/Nom") }, singleLine = true,
                    isError = error != null, supportingText = error?.let { { Text(it) } },
                )
            },
            confirmButton = {
                TextButton(enabled = error == null, onClick = {
                    flush(follow = false)
                    // le chemin affiché est celui que l'appli a retenu (normalisé), jamais la saisie brute
                    model.renameNote(id, p)?.let { path = it }
                    renaming = false
                }) { Text("Renommer") }
            },
            dismissButton = { TextButton(onClick = { renaming = false }) { Text("Annuler") } },
        )
    }
    if (confirmDelete) {
        AlertDialog(
            onDismissRequest = { confirmDelete = false },
            title = { Text("Supprimer la note ?") },
            text = { Text("« $path » sera supprimée sur tous les appareils.") },
            confirmButton = {
                TextButton(onClick = {
                    confirmDelete = false
                    saved = text // la saisie en attente part avec la note
                    model.deleteNote(id)
                    onBack?.invoke()
                }) {
                    Text("Supprimer", color = MaterialTheme.colorScheme.error)
                }
            },
            dismissButton = { TextButton(onClick = { confirmDelete = false }) { Text("Annuler") } },
        )
    }
}

/** Lecture : texte rendu par Markwon, images et whiteboards en blocs à leur taille (comme sur ordinateur). */
@Composable
private fun Rendered(model: AppModel, notePath: String, content: String, files: List<dev.mnemo.app.data.FileRow>) {
    val ctx = LocalContext.current
    val colors = MaterialTheme.colorScheme
    val markwon = remember(ctx, notePath) {
        Markwon.builder(ctx)
            .usePlugin(StrikethroughPlugin.create())
            .usePlugin(TablePlugin.create(ctx))
            .usePlugin(TaskListPlugin.create(ctx))
            .usePlugin(object : AbstractMarkwonPlugin() {
                override fun configureConfiguration(builder: MarkwonConfiguration.Builder) {
                    builder.linkResolver { view, link -> onLink(model, view.context, link, notePath) }
                }
            })
            .build()
    }
    val resolve = { target: String ->
        model.findFile(target, notePath)?.let { f ->
            when {
                f.path.endsWith(".mnemo-board", true) -> Preprocess.Embed.Board(f.path)
                Preprocess.isImage(f.path) -> Preprocess.Embed.Image(f.path)
                else -> null
            }
        }
    }
    val blocks = remember(content, notePath, files) {
        // titre de la note en tête, sauf si elle commence déjà par lui
        val title = basename(notePath)
        val src = if (content.trimStart().startsWith("# $title")) content else "# $title\n\n$content"
        Preprocess.blocks(src, resolve)
    }
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(top = 8.dp, bottom = 120.dp)) {
        blocks.forEach { b ->
            when (b) {
                is Preprocess.Block.Text -> {
                    val md = remember(b, files) { Preprocess.process(b.markdown, resolve) }
                    AndroidView(
                        factory = { c ->
                            TextView(c).apply {
                                textSize = 17f
                                setLineSpacing(0f, 1.25f)
                                setPadding(0, 8, 0, 8)
                            }
                        },
                        update = { tv ->
                            tv.setTextColor(colors.onBackground.toArgb())
                            tv.setLinkTextColor(colors.primary.toArgb())
                            markwon.setMarkdown(tv, md)
                        },
                        modifier = Modifier.fillMaxWidth(),
                    )
                }
                is Preprocess.Block.Image -> NoteImage(model, b.path, files.firstOrNull { it.path == b.path }?.localHash)
                is Preprocess.Block.Board -> BoardPreview(model, b.path)
            }
        }
    }
}

/**
 * Image d'une note : 1 pixel = 1 dp (comme Obsidian mobile), réduite à la largeur ; touchée : plein écran.
 * Décodée hors du fil principal, à une taille bornée ; [version] : empreinte du fichier (rechargée s'il change).
 */
@Composable
private fun NoteImage(model: AppModel, path: String, version: String?) {
    var image by remember(path, version) { mutableStateOf<ImageLoad>(ImageLoad.Loading) }
    LaunchedEffect(path, version) { image = model.loadImage(path) }
    val note = when (image) {
        ImageLoad.Loading -> ""
        ImageLoad.Missing -> "Image « ${basename(path)} » pas encore téléchargée"
        ImageLoad.Unreadable -> "Image « ${basename(path)} » : format non affichable sur cet appareil"
        is ImageLoad.Ready -> null
    }
    if (note != null) {
        Text(note, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(vertical = 8.dp))
        return
    }
    val bitmap = (image as ImageLoad.Ready).bitmap
    val density = androidx.compose.ui.platform.LocalDensity.current
    BoxWithConstraints(Modifier.fillMaxWidth().padding(vertical = 8.dp)) {
        val natural = with(density) { bitmap.width.toDp() * density.density }
        val w = if (natural > maxWidth) maxWidth else natural
        androidx.compose.foundation.Image(
            bitmap.asImageBitmap(),
            contentDescription = basename(path),
            modifier = Modifier.width(w).aspectRatio(bitmap.width.toFloat() / bitmap.height.coerceAtLeast(1))
                .clip(RoundedCornerShape(6.dp)).clickable { model.open(Screen.Image(path)) },
        )
    }
}

/** Aperçu d'un whiteboard intégré : le dessin entier, touché : ouvert. */
@Composable
private fun BoardPreview(model: AppModel, path: String) {
    val revision by model.revision.collectAsState()
    val sketch = remember(path, revision) {
        model.readFile(path)?.let { runCatching { dev.mnemo.app.data.Sketch.parse(String(it)) }.getOrNull() }
    }
    val ink = MaterialTheme.colorScheme.onBackground
    Box(
        Modifier.fillMaxWidth().padding(vertical = 8.dp).height(220.dp).clip(RoundedCornerShape(8.dp))
            .background(MaterialTheme.colorScheme.surface).clickable { model.open(Screen.Board(path)) },
    ) {
        androidx.compose.foundation.Canvas(Modifier.fillMaxSize()) {
            val b = sketch?.bounds() ?: return@Canvas
            val w = maxOf(b[2] - b[0], 1f); val h = maxOf(b[3] - b[1], 1f)
            val scale = minOf((size.width - 32f) / w, (size.height - 32f) / h, 3f)
            val ox = size.width / 2 - (b[0] + b[2]) / 2 * scale
            val oy = size.height / 2 - (b[1] + b[3]) / 2 * scale
            drawElements(sketch.elements, { p -> androidx.compose.ui.geometry.Offset(p.x * scale + ox, p.y * scale + oy) }, scale, ink)
        }
        Text(
            "🖊 " + basename(path).removeSuffix(".mnemo-board"),
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.align(Alignment.BottomEnd).padding(10.dp),
        )
    }
}

private fun onLink(model: AppModel, ctx: android.content.Context, link: String, notePath: String) {
    when {
        link.startsWith(Preprocess.NOTE) -> {
            val target = Preprocess.decode(link.removePrefix(Preprocess.NOTE))
            val id = model.resolveLink(target)?.id ?: model.createNote(target)
            id?.let { model.open(Screen.Note(it)) }
        }
        // intégration sans fichier affichable : on ouvre ce qui existe, sans jamais créer de note « doc.pdf »
        link.startsWith(Preprocess.EMBED) -> {
            val target = Preprocess.decode(link.removePrefix(Preprocess.EMBED))
            val file = model.findFile(target, notePath)
            val note = if (file == null) model.resolveLink(target) else null
            when {
                file != null && file.path.endsWith(".mnemo-board", true) -> model.open(Screen.Board(file.path))
                file != null && Preprocess.isImage(file.path) -> model.open(Screen.Image(file.path))
                file != null -> model.say("Ce type de fichier ne s'ouvre pas encore dans l'appli")
                note != null -> model.open(Screen.Note(note.id))
                else -> model.say("« $target » n'est pas sur cet appareil")
            }
        }
        link.startsWith(Preprocess.BOARD) -> model.open(Screen.Board(Preprocess.decode(link.removePrefix(Preprocess.BOARD))))
        link.startsWith(Preprocess.TAG) -> model.say("#" + Preprocess.decode(link.removePrefix(Preprocess.TAG)))
        else -> runCatching { ctx.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(link))) }
    }
}

@Composable
fun EmptyDetail(text: String = "Choisissez une note.") {
    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        Text(text, color = MaterialTheme.colorScheme.onSurfaceVariant, style = MaterialTheme.typography.titleMedium)
    }
}
