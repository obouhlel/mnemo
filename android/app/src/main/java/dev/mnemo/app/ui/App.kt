package dev.mnemo.app.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.VerticalDivider
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.unit.dp

/**
 * Racine de l'appli. Tablette (≥ 600 dp de large) : liste à gauche, note / whiteboard à droite.
 * Téléphone : la liste, puis l'élément ouvert en plein écran (retour = liste).
 */
@Composable
fun MnemoApp(model: AppModel) {
    val screen by model.screen.collectAsState()
    val message by model.message.collectAsState()
    val snack = remember { SnackbarHostState() }
    LaunchedEffect(message) {
        message?.let {
            snack.showSnackbar(it)
            model.clearMessage()
        }
    }
    // premier lancement : il faut une base pour récupérer les notes
    if (model.url.collectAsState().value.isBlank() && model.notes.collectAsState().value.isEmpty() && screen == Screen.List) {
        Box(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background)) {
            SettingsScreen(model, firstRun = true, onBack = null)
        }
        return
    }
    val wide = LocalConfiguration.current.screenWidthDp >= 600
    BackHandler(enabled = screen != Screen.List) { model.open(Screen.List) }
    Box(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background)) {
        if (wide) {
            Row(Modifier.fillMaxSize()) {
                Box(Modifier.width(340.dp).fillMaxHeight()) { NotesListScreen(model) }
                VerticalDivider(color = MaterialTheme.colorScheme.outline)
                Box(Modifier.fillMaxSize()) { Detail(model, screen, onBack = null) }
            }
        } else if (screen == Screen.List) {
            NotesListScreen(model)
        } else {
            Detail(model, screen, onBack = { model.open(Screen.List) })
        }
        SnackbarHost(snack, Modifier.align(Alignment.BottomCenter).navigationBarsPadding().imePadding())
    }
}

@Composable
private fun Detail(model: AppModel, screen: Screen, onBack: (() -> Unit)?) {
    when (screen) {
        // key : changer de note (tablette) repart d'un écran neuf, l'ancien enregistre sa saisie en se fermant
        is Screen.Note -> key(screen.id) { NoteScreen(model, screen.id, onBack) }
        is Screen.Board -> key(screen.path) { BoardScreen(model, screen.path, onBack) }
        is Screen.Image -> ImageScreen(model, screen.path, onBack)
        Screen.Dashboard -> DashboardScreen(model, onBack)
        Screen.Settings -> SettingsScreen(model, firstRun = false, onBack = onBack ?: { model.open(Screen.List) })
        Screen.List -> EmptyDetail()
    }
}
