package dev.mnemo.app.ui

import android.graphics.BitmapFactory
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectTransformGestures
import androidx.compose.foundation.horizontalScroll
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
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import dev.mnemo.app.data.basename
import org.json.JSONObject

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun Bar(title: String, onBack: (() -> Unit)?) {
    TopAppBar(
        title = { Text(title, maxLines = 1) },
        navigationIcon = { if (onBack != null) IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Outlined.ArrowBack, "Retour") } },
        colors = TopAppBarDefaults.topAppBarColors(containerColor = MaterialTheme.colorScheme.background),
    )
}

/** Image en plein écran, zoom et déplacement aux doigts. */
@Composable
fun ImageScreen(model: AppModel, path: String, onBack: (() -> Unit)?) {
    val bitmap = remember(path) { model.readFile(path)?.let { BitmapFactory.decodeByteArray(it, 0, it.size) } }
    var scale by remember { mutableStateOf(1f) }
    var offset by remember { mutableStateOf(androidx.compose.ui.geometry.Offset.Zero) }
    Scaffold(topBar = { Bar(basename(path), onBack) }, containerColor = MaterialTheme.colorScheme.background) { pad ->
        Box(
            Modifier.padding(pad).fillMaxSize().pointerInput(Unit) {
                detectTransformGestures { _, pan, zoom, _ ->
                    scale = (scale * zoom).coerceIn(1f, 8f); offset += pan
                }
            },
            contentAlignment = Alignment.Center,
        ) {
            if (bitmap == null) Text("Image pas encore téléchargée.", color = MaterialTheme.colorScheme.onSurfaceVariant)
            else Image(
                bitmap.asImageBitmap(), basename(path),
                Modifier.fillMaxSize().graphicsLayer(scaleX = scale, scaleY = scale, translationX = offset.x, translationY = offset.y),
            )
        }
    }
}

private val STATUS = listOf(
    "todo" to ("À faire" to Color(0xFF9A9A9A)),
    "doing" to ("En cours" to Color(0xFF3B9EF5)),
    "blocked" to ("Bloqué" to Color(0xFFFB464C)),
    "review" to ("En revue" to Color(0xFFE9973F)),
    "done" to ("Terminé" to Color(0xFF44CF6E)),
)

/** Tableau de bord en lecture : projets, tickets par statut (avec leur issue / PR GitHub). */
@Composable
fun DashboardScreen(model: AppModel, onBack: (() -> Unit)?) {
    val revision by model.revision.collectAsState()
    val board = remember(revision) { model.boardJson()?.let { runCatching { JSONObject(it) }.getOrNull() } }
    var project by rememberSaveable { mutableStateOf<String?>(null) }
    Scaffold(topBar = { Bar("Tableau de bord", onBack) }, containerColor = MaterialTheme.colorScheme.background) { pad ->
        if (board == null) {
            Box(Modifier.padding(pad).fillMaxSize(), contentAlignment = Alignment.Center) {
                Text("Le tableau de bord arrive avec la première synchronisation.", color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            return@Scaffold
        }
        val projects = board.getJSONArray("projects").let { a -> (0 until a.length()).map { a.getJSONObject(it) } }
        val tickets = board.getJSONArray("tickets").let { a -> (0 until a.length()).map { a.getJSONObject(it) } }
            .filter { project == null || it.getString("project") == project }
        Column(Modifier.padding(pad).fillMaxSize()) {
            Row(Modifier.horizontalScroll(rememberScrollState()).padding(horizontal = 12.dp, vertical = 6.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                FilterChip(project == null, { project = null }, label = { Text("Tous") })
                projects.forEach { p ->
                    FilterChip(project == p.getString("id"), { project = p.getString("id") }, label = { Text(p.getString("name")) })
                }
            }
            val done = tickets.count { it.getString("status") == "done" }
            Text(
                "$done / ${tickets.size} terminés",
                Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            LazyColumn(Modifier.fillMaxSize()) {
                STATUS.forEach { (key, info) ->
                    val group = tickets.filter { it.getString("status") == key }
                    if (group.isEmpty()) return@forEach
                    item(key = "h:$key") {
                        Text(
                            "${info.first}  ${group.size}",
                            Modifier.padding(start = 16.dp, top = 14.dp, bottom = 4.dp),
                            color = info.second, fontWeight = FontWeight.SemiBold,
                        )
                    }
                    items(group, key = { it.getString("id") }) { t ->
                        Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                            Box(Modifier.size(8.dp).background(info.second, CircleShape))
                            Column(Modifier.padding(start = 12.dp).weight(1f)) {
                                Text(t.getString("title"), style = MaterialTheme.typography.bodyLarge)
                                val tags = t.getJSONArray("tags").let { a -> (0 until a.length()).map { a.getString(it) } }
                                val gh = if (t.isNull("github")) null else (if (t.optBoolean("pr")) "PR #" else "#") + t.getInt("github")
                                val meta = listOfNotNull(gh) + tags
                                if (meta.isNotEmpty()) Text(meta.joinToString("  ·  "), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                            }
                        }
                    }
                }
            }
        }
    }
}

/** Réglages : adresse de la base de synchronisation (aussi l'écran du premier lancement). */
@Composable
fun SettingsScreen(model: AppModel, firstRun: Boolean, onBack: (() -> Unit)?) {
    val sync by model.sync.collectAsState()
    val pending by model.pending.collectAsState()
    var url by rememberSaveable { mutableStateOf(model.dbUrl()) }
    var show by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    Scaffold(topBar = { if (!firstRun) Bar("Paramètres", onBack) }, containerColor = MaterialTheme.colorScheme.background) { pad ->
        Box(Modifier.padding(pad).fillMaxSize(), contentAlignment = Alignment.TopCenter) {
            Column(Modifier.widthIn(max = 560.dp).fillMaxWidth().verticalScroll(rememberScrollState()).padding(24.dp)) {
                if (firstRun) {
                    Spacer(Modifier.height(32.dp))
                    Text("Mnemo", style = MaterialTheme.typography.displaySmall, fontWeight = FontWeight.SemiBold)
                    Spacer(Modifier.height(8.dp))
                    Text("Vos notes Mnemo sur téléphone et tablette.", color = MaterialTheme.colorScheme.onSurfaceVariant)
                    Spacer(Modifier.height(24.dp))
                }
                Text("Base de synchronisation", style = MaterialTheme.typography.titleMedium)
                Spacer(Modifier.height(6.dp))
                Text(
                    "La même adresse que dans Mnemo sur ordinateur (Paramètres › Synchronisation). La base doit être joignable " +
                        "depuis cet appareil : base en ligne (Neon, Supabase…), serveur exposé ou VPN. Ajoutez ?sslmode=require " +
                        "pour une base distante. Les notes restent lisibles et modifiables hors ligne.",
                    style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Spacer(Modifier.height(12.dp))
                OutlinedTextField(
                    url, { url = it; error = null },
                    label = { Text("postgres://utilisateur:motdepasse@hôte:5432/base") },
                    singleLine = true,
                    visualTransformation = if (show) VisualTransformation.None else PasswordVisualTransformation(),
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
                    modifier = Modifier.fillMaxWidth(),
                )
                TextButton(onClick = { show = !show }) { Text(if (show) "Masquer l'adresse" else "Afficher l'adresse") }
                error?.let { Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall) }
                Spacer(Modifier.height(8.dp))
                Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    Button(enabled = !busy && url.isNotBlank(), onClick = {
                        busy = true
                        model.connect(url) { err ->
                            busy = false; error = err
                            if (err == null) {
                                model.syncNow()
                                if (!firstRun) model.say("Base connectée")
                                model.open(Screen.List)
                            }
                        }
                    }) { Text(if (busy) "Connexion…" else "Connecter") }
                    if (!firstRun && model.dbUrl().isNotBlank()) OutlinedButton(onClick = { model.syncNow() }) { Text("Synchroniser") }
                }
                if (!firstRun) {
                    Spacer(Modifier.height(24.dp))
                    val state = when (val s = sync) {
                        SyncState.LocalOnly -> "Aucune base : notes sur cet appareil uniquement"
                        SyncState.Syncing -> "Synchronisation…"
                        is SyncState.Synced -> "Synchronisé à " + java.text.SimpleDateFormat("HH:mm:ss", java.util.Locale.FRANCE).format(java.util.Date(s.at))
                        is SyncState.Offline -> "Hors ligne : ${s.error}"
                    }
                    Text(state, style = MaterialTheme.typography.bodyMedium)
                    if (pending > 0) Text("$pending modification(s) en attente d'envoi", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    if (model.dbUrl().isNotBlank()) {
                        Spacer(Modifier.height(16.dp))
                        TextButton(onClick = { model.disconnect(); url = "" }) {
                            Text("Arrêter la synchronisation", color = MaterialTheme.colorScheme.error)
                        }
                    }
                }
                if (firstRun) {
                    Spacer(Modifier.height(24.dp))
                    TextButton(onClick = { model.createNote("Bienvenue")?.let { model.open(Screen.Note(it)) } }) {
                        Text("Continuer sans base (notes sur cet appareil)")
                    }
                }
            }
        }
    }
}
