package dev.mnemo.app.data

import org.json.JSONArray
import org.json.JSONObject
import kotlin.math.hypot
import kotlin.math.max
import kotlin.math.min

/**
 * Whiteboard `.mnemo-board` : même format JSON que Mnemo sur ordinateur (src/sketch.rs).
 * Les éléments inconnus (format plus récent) sont conservés tels quels à l'enregistrement.
 */
sealed class Element {
    abstract val color: String

    data class Stroke(override val color: String, val width: Float, val points: List<Pt>) : Element()
    data class Line(override val color: String, val width: Float, val points: List<Pt>, val arrow: Boolean) : Element()
    data class Shape(override val color: String, val width: Float, val x: Float, val y: Float, val w: Float, val h: Float, val ellipse: Boolean) : Element()
    data class Text(override val color: String, val size: Float, val x: Float, val y: Float, val text: String) : Element()
    data class Unknown(val json: JSONObject) : Element() {
        override val color: String get() = json.optString("color", "#888888")
    }

    /** (min x, min y, max x, max y) */
    fun bounds(): FloatArray = when (this) {
        is Stroke -> pointsBounds(points, width)
        is Line -> pointsBounds(points, width)
        is Shape -> floatArrayOf(min(x, x + w), min(y, y + h), max(x, x + w), max(y, y + h))
        is Text -> {
            val lines = text.lines()
            val w = (lines.maxOfOrNull { it.length } ?: 1) * size * 0.55f
            floatArrayOf(x, y, x + max(w, size), y + lines.size.coerceAtLeast(1) * size * 1.25f)
        }
        is Unknown -> floatArrayOf(0f, 0f, 0f, 0f)
    }

    fun hit(p: Pt, tol: Float): Boolean = when (this) {
        is Stroke -> nearPolyline(points, p, width / 2 + tol)
        is Line -> nearPolyline(points, p, width / 2 + tol)
        is Shape -> {
            val b = bounds()
            if (ellipse) {
                val cx = (b[0] + b[2]) / 2; val cy = (b[1] + b[3]) / 2
                val rx = (b[2] - b[0]) / 2 + tol; val ry = (b[3] - b[1]) / 2 + tol
                ((p.x - cx) / rx).let { it * it } + ((p.y - cy) / ry).let { it * it } <= 1f
            } else p.x >= b[0] - tol && p.x <= b[2] + tol && p.y >= b[1] - tol && p.y <= b[3] + tol
        }
        is Text -> bounds().let { b -> p.x >= b[0] - tol && p.x <= b[2] + tol && p.y >= b[1] - tol && p.y <= b[3] + tol }
        is Unknown -> false
    }

    fun toJson(): JSONObject = when (this) {
        is Stroke -> JSONObject().put("type", "stroke").put("color", color).put("width", width.toDouble()).put("points", pointsJson(points))
        is Line -> JSONObject().put("type", if (arrow) "arrow" else "line").put("color", color).put("width", width.toDouble()).put("points", pointsJson(points))
        is Shape -> JSONObject().put("type", if (ellipse) "ellipse" else "rect").put("color", color).put("width", width.toDouble())
            .put("x", x.toDouble()).put("y", y.toDouble()).put("w", w.toDouble()).put("h", h.toDouble())
        is Text -> JSONObject().put("type", "text").put("color", color).put("size", size.toDouble())
            .put("x", x.toDouble()).put("y", y.toDouble()).put("text", text)
        is Unknown -> json
    }

    companion object {
        fun fromJson(o: JSONObject): Element {
            val color = o.optString("color", "#1e1e1e")
            fun pts(): List<Pt> {
                val a = o.optJSONArray("points") ?: return emptyList()
                return (0 until a.length()).mapNotNull { i ->
                    a.optJSONArray(i)?.let { Pt(it.optDouble(0).toFloat(), it.optDouble(1).toFloat()) }
                }
            }
            fun f(k: String) = o.optDouble(k, 0.0).toFloat()
            return when (o.optString("type")) {
                "stroke" -> Stroke(color, f("width"), pts())
                "line" -> Line(color, f("width"), pts(), false)
                "arrow" -> Line(color, f("width"), pts(), true)
                "rect" -> Shape(color, f("width"), f("x"), f("y"), f("w"), f("h"), false)
                "ellipse" -> Shape(color, f("width"), f("x"), f("y"), f("w"), f("h"), true)
                "text" -> Text(color, f("size"), f("x"), f("y"), o.optString("text"))
                else -> Unknown(o)
            }
        }
    }
}

data class Pt(val x: Float, val y: Float)

data class Sketch(val elements: List<Element> = emptyList()) {
    fun toJson(): String {
        val arr = JSONArray()
        elements.forEach { arr.put(it.toJson()) }
        return JSONObject().put("version", 1).put("elements", arr).toString(2)
    }

    fun bounds(): FloatArray? = elements.filter { it !is Element.Unknown }.map { it.bounds() }.reduceOrNull { a, b ->
        floatArrayOf(min(a[0], b[0]), min(a[1], b[1]), max(a[2], b[2]), max(a[3], b[3]))
    }

    fun hit(p: Pt, tol: Float): Int? = elements.indices.reversed().firstOrNull { elements[it].hit(p, tol) }

    companion object {
        fun parse(s: String): Sketch {
            if (s.isBlank()) return Sketch()
            val arr = JSONObject(s).optJSONArray("elements") ?: return Sketch()
            return Sketch((0 until arr.length()).mapNotNull { arr.optJSONObject(it)?.let(Element::fromJson) })
        }
    }
}

/** Couleur « encre » : suit le thème (claire sur fond sombre), comme sur ordinateur. */
const val INK = "#1e1e1e"
val COLORS = listOf(INK, "#e03131", "#1971c2", "#2f9e44", "#f08c00", "#9c36b5")

private fun pointsJson(points: List<Pt>): JSONArray {
    val a = JSONArray()
    points.forEach { a.put(JSONArray().put(it.x.toDouble()).put(it.y.toDouble())) }
    return a
}

private fun pointsBounds(points: List<Pt>, width: Float): FloatArray {
    if (points.isEmpty()) return floatArrayOf(0f, 0f, 0f, 0f)
    val m = width / 2
    return floatArrayOf(points.minOf { it.x } - m, points.minOf { it.y } - m, points.maxOf { it.x } + m, points.maxOf { it.y } + m)
}

private fun distToSegment(p: Pt, a: Pt, b: Pt): Float {
    val dx = b.x - a.x; val dy = b.y - a.y
    val len2 = dx * dx + dy * dy
    val t = if (len2 == 0f) 0f else (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).coerceIn(0f, 1f)
    return hypot(p.x - (a.x + t * dx), p.y - (a.y + t * dy))
}

private fun nearPolyline(points: List<Pt>, p: Pt, reach: Float): Boolean = when (points.size) {
    0 -> false
    1 -> distToSegment(p, points[0], points[0]) <= reach
    else -> points.zipWithNext().any { (a, b) -> distToSegment(p, a, b) <= reach }
}

/** Simplification d'un trait (Ramer-Douglas-Peucker), comme sur ordinateur. */
fun simplify(points: List<Pt>, epsilon: Float): List<Pt> {
    if (points.size < 3) return points
    val a = points.first(); val b = points.last()
    var idx = 0; var dmax = 0f
    for (i in 1 until points.size - 1) {
        val d = distToSegment(points[i], a, b)
        if (d > dmax) { idx = i; dmax = d }
    }
    return if (dmax > epsilon) simplify(points.subList(0, idx + 1), epsilon).dropLast(1) + simplify(points.subList(idx, points.size), epsilon)
    else listOf(a, b)
}
