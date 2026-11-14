package dev.mnemo.app.ui

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.calculateCentroid
import androidx.compose.foundation.gestures.calculatePan
import androidx.compose.foundation.gestures.calculateZoom
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.automirrored.outlined.Redo
import androidx.compose.material.icons.automirrored.outlined.Undo
import androidx.compose.material.icons.outlined.CenterFocusStrong
import androidx.compose.material.icons.outlined.Circle
import androidx.compose.material.icons.outlined.CropSquare
import androidx.compose.material.icons.outlined.Draw
import androidx.compose.material.icons.outlined.HorizontalRule
import androidx.compose.material.icons.outlined.NorthEast
import androidx.compose.material.icons.outlined.TextFields
import androidx.compose.material.icons.outlined.AutoFixNormal
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.nativeCanvas
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import dev.mnemo.app.data.COLORS
import dev.mnemo.app.data.Element
import dev.mnemo.app.data.INK
import dev.mnemo.app.data.Pt
import dev.mnemo.app.data.Sketch
import dev.mnemo.app.data.basename
import dev.mnemo.app.data.fileHash
import dev.mnemo.app.data.simplify
import kotlin.math.max
import kotlin.math.min

private enum class Tool(val icon: ImageVector, val label: String) {
    Pen(Icons.Outlined.Draw, "Stylo"),
    Line(Icons.Outlined.HorizontalRule, "Ligne"),
    Arrow(Icons.Outlined.NorthEast, "Flèche"),
    Rect(Icons.Outlined.CropSquare, "Rectangle"),
    Ellipse(Icons.Outlined.Circle, "Ellipse"),
    Text(Icons.Outlined.TextFields, "Texte"),
    Eraser(Icons.Outlined.AutoFixNormal, "Gomme"),
}

private val WIDTHS = listOf(2f, 4f, 8f)

/** Couleur d'affichage : l'« encre » suit le thème. */
fun displayColor(c: String, ink: Color): Color {
    if (c.equals(INK, true)) return ink
    return runCatching { Color(android.graphics.Color.parseColor(c)) }.getOrDefault(Color.Gray)
}

/** Dessine les éléments ; `toScreen` place un point du dessin, `scale` = taille d'une unité. */
fun DrawScope.drawElements(elements: List<Element>, toScreen: (Pt) -> Offset, scale: Float, ink: Color) {
    for (el in elements) {
        val color = displayColor(el.color, ink)
        when (el) {
            is Element.Stroke -> {
                val w = max(el.width * scale, 1f)
                if (el.points.size == 1) {
                    drawCircle(color, w / 2, toScreen(el.points[0]))
                } else if (el.points.isNotEmpty()) {
                    val path = Path()
                    el.points.forEachIndexed { i, p -> toScreen(p).let { if (i == 0) path.moveTo(it.x, it.y) else path.lineTo(it.x, it.y) } }
                    drawPath(path, color, style = Stroke(w, cap = StrokeCap.Round, join = StrokeJoin.Round))
                }
            }
            is Element.Line -> if (el.points.size >= 2) {
                val w = max(el.width * scale, 1f)
                val a = toScreen(el.points.first()); val b = toScreen(el.points.last())
                drawLine(color, a, b, w, StrokeCap.Round)
                if (el.arrow) {
                    val d = (b - a).let { v -> v / max(v.getDistance(), 0.001f) }
                    val head = (10f + el.width * 2f) * scale
                    val n = Offset(-d.y, d.x)
                    drawLine(color, b, b - d * head + n * head * 0.5f, w, StrokeCap.Round)
                    drawLine(color, b, b - d * head - n * head * 0.5f, w, StrokeCap.Round)
                }
            }
            is Element.Shape -> {
                val b = el.bounds()
                val tl = toScreen(Pt(b[0], b[1])); val br = toScreen(Pt(b[2], b[3]))
                val style = Stroke(max(el.width * scale, 1f))
                if (el.ellipse) drawOval(color, tl, Size(br.x - tl.x, br.y - tl.y), style = style)
                else drawRect(color, tl, Size(br.x - tl.x, br.y - tl.y), style = style)
            }
            is Element.Text -> {
                val paint = android.graphics.Paint().apply {
                    this.color = color.toArgb(); textSize = max(el.size * scale, 1f); isAntiAlias = true
                }
                val o = toScreen(Pt(el.x, el.y))
                el.text.lines().forEachIndexed { i, line ->
                    drawContext.canvas.nativeCanvas.drawText(line, o.x, o.y + paint.textSize * (0.9f + i * 1.25f), paint)
                }
            }
            is Element.Unknown -> {}
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun BoardScreen(model: AppModel, path: String, onBack: (() -> Unit)?) {
    val revision by model.revision.collectAsState()
    val ink = MaterialTheme.colorScheme.onBackground
    val elements = remember(path) { mutableStateListOf<Element>() }
    val undo = remember(path) { mutableListOf<List<Element>>() }
    val redo = remember(path) { mutableListOf<List<Element>>() }
    var tool by remember { mutableStateOf(Tool.Pen) }
    var color by remember { mutableStateOf(INK) }
    var width by remember { mutableStateOf(WIDTHS[0]) }
    var pan by remember(path) { mutableStateOf(Offset.Zero) }
    var zoom by remember(path) { mutableStateOf(1f) }
    var size by remember { mutableStateOf(IntSize.Zero) }
    var fitted by remember(path) { mutableStateOf(false) }
    var drawing by remember { mutableStateOf<Element?>(null) }
    var textAt by remember { mutableStateOf<Pt?>(null) }
    var undoCount by remember { mutableStateOf(0) } // pour rafraîchir l'état des boutons
    // Tant que le dessin n'est pas chargé (pas téléchargé, illisible), on ne dessine pas et surtout on n'enregistre
    // pas : un fichier illisible n'est pas un dessin vide, le premier trait l'écraserait.
    var problem by remember(path) { mutableStateOf<String?>("Chargement…") }
    /** empreinte du fichier tel que chargé ou enregistré ici : s'il a changé ailleurs, l'enregistrement part dans une copie */
    var loadedHash by remember(path) { mutableStateOf<String?>(null) }

    fun fit() {
        val b = Sketch(elements.toList()).bounds()
        if (b == null || size == IntSize.Zero) { zoom = 1f; pan = Offset.Zero; return }
        val w = max(b[2] - b[0], 1f); val h = max(b[3] - b[1], 1f)
        zoom = min((size.width - 80f) / w, (size.height - 160f) / h).coerceIn(0.1f, 3f)
        pan = Offset(size.width / (2 * zoom) - (b[0] + b[2]) / 2, size.height / (2 * zoom) - (b[1] + b[3]) / 2)
    }

    fun save() {
        if (problem != null) return
        val bytes = Sketch(elements.toList()).toJson().toByteArray()
        val target = model.writeFile(path, bytes, loadedHash)
        loadedHash = fileHash(bytes)
        if (target != path) model.open(Screen.Board(target)) // modifié ailleurs entre-temps : on continue dans la copie
    }

    fun commit(change: () -> Unit) {
        undo += elements.toList()
        redo.clear()
        change()
        undoCount++
        save()
    }

    // chargement, et rechargement quand le whiteboard change sur un autre appareil
    LaunchedEffect(path, revision) {
        if (drawing != null) return@LaunchedEffect
        val bytes = model.readFile(path)
        if (bytes == null) {
            problem = if (model.knowsFile(path)) "Ce whiteboard n'est pas encore téléchargé." else "Ce whiteboard n'existe plus."
            return@LaunchedEffect
        }
        val hash = fileHash(bytes)
        if (hash == loadedHash) return@LaunchedEffect
        val sk = runCatching { Sketch.parse(String(bytes)) }.getOrNull()
        if (sk == null) {
            problem = "Whiteboard illisible (fichier incomplet ou endommagé). Il est laissé tel quel : rien ne sera enregistré par-dessus."
            return@LaunchedEffect
        }
        elements.clear(); elements.addAll(sk.elements)
        loadedHash = hash
        problem = null
    }
    // cadrage sur le dessin à l'ouverture, dès que la taille de l'écran et le dessin sont connus
    LaunchedEffect(size, elements.size) {
        if (!fitted && size != IntSize.Zero && elements.isNotEmpty()) { fit(); fitted = true }
    }

    val toScreen = { p: Pt -> Offset((p.x + pan.x) * zoom, (p.y + pan.y) * zoom) }
    val toWorld = { o: Offset -> Pt(o.x / zoom - pan.x, o.y / zoom - pan.y) }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(basename(path).removeSuffix(".mnemo-board"), maxLines = 1) },
                navigationIcon = { if (onBack != null) IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Outlined.ArrowBack, "Retour") } },
                colors = TopAppBarDefaults.topAppBarColors(containerColor = MaterialTheme.colorScheme.background),
            )
        },
        containerColor = MaterialTheme.colorScheme.background,
    ) { pad ->
        Box(Modifier.padding(pad).fillMaxSize()) {
            Canvas(
                Modifier.fillMaxSize()
                    .onSizeChanged { size = it }
                    .pointerInput(tool, color, width, problem) {
                        if (problem != null) return@pointerInput
                        awaitEachGesture {
                            val down = awaitFirstDown()
                            val start = toWorld(down.position)
                            var multi = false
                            var current: Element? = when (tool) {
                                Tool.Pen -> Element.Stroke(color, width, listOf(start))
                                Tool.Line -> Element.Line(color, width, listOf(start, start), false)
                                Tool.Arrow -> Element.Line(color, width, listOf(start, start), true)
                                Tool.Rect -> Element.Shape(color, width, start.x, start.y, 0f, 0f, false)
                                Tool.Ellipse -> Element.Shape(color, width, start.x, start.y, 0f, 0f, true)
                                else -> null
                            }
                            var erased = false
                            fun erase(p: Pt) {
                                val i = Sketch(elements.toList()).hit(p, 12f / zoom) ?: return
                                if (!erased) { undo += elements.toList(); redo.clear(); erased = true }
                                elements.removeAt(i)
                            }
                            if (tool == Tool.Eraser) erase(start)
                            drawing = current
                            while (true) {
                                val event = awaitPointerEvent()
                                val pressed = event.changes.filter { it.pressed }
                                if (pressed.isEmpty()) break
                                if (pressed.size >= 2) {
                                    // deux doigts : déplacement et zoom de la vue, le trait en cours est abandonné
                                    multi = true
                                    current = null; drawing = null
                                    val z = event.calculateZoom()
                                    val c = event.calculateCentroid()
                                    val before = toWorld(c)
                                    zoom = (zoom * z).coerceIn(0.05f, 8f)
                                    pan = Offset(c.x / zoom - before.x, c.y / zoom - before.y) + event.calculatePan() / zoom
                                    event.changes.forEach { it.consume() }
                                    continue
                                }
                                if (multi) continue
                                val p = toWorld(pressed[0].position)
                                pressed[0].consume()
                                when (val el = current) {
                                    is Element.Stroke -> {
                                        val last = el.points.last()
                                        if ((p.x - last.x) * (p.x - last.x) + (p.y - last.y) * (p.y - last.y) > (1.5f / zoom) * (1.5f / zoom)) {
                                            current = el.copy(points = el.points + p)
                                        }
                                    }
                                    is Element.Line -> current = el.copy(points = listOf(el.points.first(), p))
                                    is Element.Shape -> current = el.copy(w = p.x - el.x, h = p.y - el.y)
                                    else -> if (tool == Tool.Eraser) erase(p)
                                }
                                drawing = current
                            }
                            val done = current
                            drawing = null
                            when {
                                erased -> { undoCount++; save() }
                                tool == Tool.Text && !multi -> textAt = start
                                done is Element.Stroke -> commit { elements += done.copy(points = simplify(done.points, 0.6f / zoom)) }
                                done != null -> {
                                    val b = done.bounds()
                                    if (max(b[2] - b[0], b[3] - b[1]) * zoom > 4f) commit { elements += done }
                                }
                            }
                        }
                    },
            ) {
                // grille de points
                val step = 24f * zoom
                if (step > 8f) {
                    val dot = Color.Gray.copy(alpha = 0.25f)
                    var y = (pan.y * zoom).mod(step)
                    while (y < this.size.height) {
                        var x = (pan.x * zoom).mod(step)
                        while (x < this.size.width) {
                            drawCircle(dot, 1.5f, Offset(x, y)); x += step
                        }
                        y += step
                    }
                }
                drawElements(elements, toScreen, zoom, ink)
                drawing?.let { drawElements(listOf(it), toScreen, zoom, ink) }
            }

            problem?.let {
                Text(
                    it, Modifier.align(Alignment.Center).padding(32.dp),
                    color = MaterialTheme.colorScheme.onSurfaceVariant, style = MaterialTheme.typography.titleMedium,
                )
            }
            // barre d'outils (en bas, défilable sur petit écran)
            Surface(
                Modifier.align(Alignment.BottomCenter).padding(12.dp),
                shape = RoundedCornerShape(16.dp),
                color = MaterialTheme.colorScheme.surface,
                shadowElevation = 4.dp,
            ) {
                Row(
                    Modifier.horizontalScroll(rememberScrollState()).padding(horizontal = 6.dp, vertical = 2.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(2.dp),
                ) {
                    Tool.entries.forEach { t ->
                        IconButton(
                            onClick = { tool = t },
                            modifier = Modifier.background(if (tool == t) MaterialTheme.colorScheme.surfaceVariant else Color.Transparent, CircleShape),
                        ) { Icon(t.icon, t.label, tint = if (tool == t) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant) }
                    }
                    Spacer(Modifier.width(6.dp))
                    COLORS.forEach { c ->
                        Box(
                            Modifier.size(32.dp).clickable { color = c }
                                .then(if (color == c) Modifier.border(2.dp, MaterialTheme.colorScheme.primary, CircleShape) else Modifier)
                                .padding(7.dp).background(displayColor(c, ink), CircleShape),
                        )
                    }
                    Spacer(Modifier.width(6.dp))
                    WIDTHS.forEach { w ->
                        Box(
                            Modifier.size(32.dp).clickable { width = w }
                                .background(if (width == w) MaterialTheme.colorScheme.surfaceVariant else Color.Transparent, CircleShape),
                            contentAlignment = Alignment.Center,
                        ) { Box(Modifier.size(16.dp, (w * 0.8f).dp).background(MaterialTheme.colorScheme.onSurface, CircleShape)) }
                    }
                    Spacer(Modifier.width(6.dp))
                    IconButton(onClick = {
                        if (undo.isNotEmpty()) {
                            redo += elements.toList(); val prev = undo.removeAt(undo.size - 1)
                            elements.clear(); elements.addAll(prev); undoCount++; save()
                        }
                    }, enabled = undoCount >= 0 && undo.isNotEmpty()) { Icon(Icons.AutoMirrored.Outlined.Undo, "Annuler") }
                    IconButton(onClick = {
                        if (redo.isNotEmpty()) {
                            undo += elements.toList(); val next = redo.removeAt(redo.size - 1)
                            elements.clear(); elements.addAll(next); undoCount++; save()
                        }
                    }, enabled = undoCount >= 0 && redo.isNotEmpty()) { Icon(Icons.AutoMirrored.Outlined.Redo, "Rétablir") }
                    IconButton(onClick = { fit() }) { Icon(Icons.Outlined.CenterFocusStrong, "Tout afficher") }
                }
            }
        }
    }

    textAt?.let { at ->
        var t by remember { mutableStateOf("") }
        AlertDialog(
            onDismissRequest = { textAt = null },
            title = { Text("Texte") },
            text = { OutlinedTextField(t, { t = it }) },
            confirmButton = {
                TextButton(onClick = {
                    if (t.isNotBlank()) commit { elements += Element.Text(color, 12f + width * 3f, at.x, at.y, t.trimEnd()) }
                    textAt = null
                }) { Text("Ajouter") }
            },
            dismissButton = { TextButton(onClick = { textAt = null }) { Text("Annuler") } },
        )
    }
}
