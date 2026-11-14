package dev.mnemo.app.ui

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color

/** Mêmes couleurs que Mnemo sur ordinateur (thème par défaut d'Obsidian). */
val Accent = Color(0xFF8B6CEF)

private val Dark = darkColorScheme(
    primary = Accent,
    onPrimary = Color.White,
    background = Color(0xFF1E1E1E),
    surface = Color(0xFF262626),
    surfaceVariant = Color(0xFF2E2E2E),
    onBackground = Color(0xFFDADADA),
    onSurface = Color(0xFFDADADA),
    onSurfaceVariant = Color(0xFFA0A0A0),
    outline = Color(0xFF3F3F3F),
)

private val Light = lightColorScheme(
    primary = Color(0xFF7052E6),
    onPrimary = Color.White,
    background = Color(0xFFFFFFFF),
    surface = Color(0xFFF6F6F6),
    surfaceVariant = Color(0xFFEDEDED),
    onBackground = Color(0xFF222222),
    onSurface = Color(0xFF222222),
    onSurfaceVariant = Color(0xFF5C5C5C),
    outline = Color(0xFFE0E0E0),
)

@Composable
fun MnemoTheme(dark: Boolean = isSystemInDarkTheme(), content: @Composable () -> Unit) {
    MaterialTheme(colorScheme = if (dark) Dark else Light, content = content)
}
