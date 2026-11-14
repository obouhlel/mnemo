package dev.mnemo.app.markdown

/**
 * Transforme le Markdown de Mnemo / Obsidian pour l'affichage (même règles que src/markdown.rs) :
 * `[[note|alias]]` → lien `mnemo://note/…`, `#tag` → lien `mnemo://tag/…`,
 * `![[image.png]]` / `![](chemin.png)` → image `file://…` si le fichier est disponible,
 * `![[croquis.mnemo-board]]` → lien `mnemo://board/…`. Le code (blocs et `en ligne`) n'est pas touché.
 */
object Preprocess {
    const val NOTE = "mnemo://note/"
    const val TAG = "mnemo://tag/"
    const val BOARD = "mnemo://board/"

    sealed class Embed {
        data class Image(val uri: String) : Embed()
        data class Board(val path: String) : Embed()
    }

    private val imageExts = setOf("png", "jpg", "jpeg", "gif", "webp", "bmp")

    fun isImage(path: String): Boolean = path.substringAfterLast('.', "").lowercase() in imageExts

    fun encode(s: String): String = java.net.URLEncoder.encode(s, "UTF-8").replace("+", "%20")

    fun decode(s: String): String = java.net.URLDecoder.decode(s, "UTF-8")

    /** Cible et texte affiché de l'intérieur de `[[…]]`. */
    fun parseWikilink(inner: String): Pair<String, String> {
        val targetPart = inner.substringBefore('|')
        val alias = if (inner.contains('|')) inner.substringAfter('|').trim() else ""
        val target = targetPart.substringBefore('#').trim()
        val display = alias.ifEmpty { targetPart.trim().replace("#", " > ") }
        return target to display
    }

    fun process(src: String, resolve: (String) -> Embed? = { null }): String {
        val out = StringBuilder(src.length + 64)
        var fence: String? = null
        for (line in src.split('\n').let { parts -> parts.mapIndexed { i, p -> if (i < parts.size - 1) "$p\n" else p } }) {
            val t = line.trimStart()
            val f = fence
            if (f != null) {
                if (t.startsWith(f)) fence = null
                out.append(line)
                continue
            }
            if (t.startsWith("```") || t.startsWith("~~~")) {
                val ch = t[0]
                fence = ch.toString().repeat(t.takeWhile { it == ch }.length)
                out.append(line)
                continue
            }
            if (line.startsWith("    ") || line.startsWith("\t")) {
                out.append(line)
                continue
            }
            transformLine(line, out, resolve)
        }
        return out.toString()
    }

    private fun embedMarkdown(target: String, label: String, resolve: (String) -> Embed?): String? {
        val l = label.replace("[", "\\[").replace("]", "\\]")
        return when (val e = resolve(target)) {
            is Embed.Image -> "![$l](${e.uri.replace(" ", "%20")})"
            is Embed.Board -> "[🖊 $l]($BOARD${encode(e.path)})"
            null -> null
        }
    }

    private fun transformLine(line: String, out: StringBuilder, resolve: (String) -> Embed?) {
        var i = 0
        var last = 0
        while (i < line.length) {
            val c = line[i]
            when {
                c == '`' -> {
                    val n = line.substring(i).takeWhile { it == '`' }.length
                    val end = line.indexOf("`".repeat(n), i + n)
                    i = if (end >= 0) end + n else i + n
                }
                c == '[' && line.getOrNull(i + 1) == '[' -> {
                    val end = line.indexOf("]]", i + 2)
                    val inner = if (end >= 0) line.substring(i + 2, end) else ""
                    if (end >= 0 && inner.isNotEmpty() && !inner.contains('[')) {
                        val embed = i > 0 && line[i - 1] == '!'
                        val start = if (embed) i - 1 else i
                        out.append(line, last, start)
                        val (target, display) = parseWikilink(inner)
                        val md = if (embed) embedMarkdown(inner.substringBefore('|').trim(), target, resolve) else null
                        out.append(md ?: "[${display.replace("[", "\\[").replace("]", "\\]")}]($NOTE${encode(target)})")
                        i = end + 2
                        last = i
                    } else {
                        i++
                    }
                }
                c == '!' && line.getOrNull(i + 1) == '[' && line.getOrNull(i + 2) != '[' -> {
                    val close = line.indexOf("](", i + 2)
                    val end = if (close >= 0) line.indexOf(')', close + 2) else -1
                    if (close >= 0 && end >= 0) {
                        val raw = line.substring(close + 2, end).trim().removePrefix("<").removeSuffix(">")
                        val external = raw.contains("://") || raw.startsWith("data:")
                        val md = if (!external) embedMarkdown(raw.replace("%20", " "), line.substring(i + 2, close), resolve) else null
                        if (md != null) {
                            out.append(line, last, i)
                            out.append(md)
                            i = end + 1
                            last = i
                            continue
                        }
                    }
                    i++
                }
                c == '#' && (i == 0 || line[i - 1].isWhitespace()) -> {
                    val first = line.getOrNull(i + 1)
                    if (first != null && (first.isLetter() || first == '_')) {
                        val tag = line.substring(i + 1).takeWhile { it.isLetterOrDigit() || it == '_' || it == '-' || it == '/' }
                        out.append(line, last, i)
                        out.append("[#$tag]($TAG${encode(tag)})")
                        i += 1 + tag.length
                        last = i
                    } else {
                        i++
                    }
                }
                else -> i++
            }
        }
        out.append(line, last, line.length)
    }

    /** Morceau d'une note pour l'affichage : texte Markdown, ou intégration seule sur sa ligne. */
    sealed class Block {
        data class Text(val markdown: String) : Block()
        data class Image(val path: String) : Block()
        data class Board(val path: String) : Block()
    }

    /** Cible d'une ligne qui n'est qu'une intégration (`![[x]]` ou `![…](x)`), sinon null. */
    fun embedTarget(line: String): String? {
        val t = line.trim()
        if (t.startsWith("![[") && t.endsWith("]]") && t.indexOf("]]") == t.length - 2) {
            return t.substring(3, t.length - 2).substringBefore('|').trim()
        }
        if (t.startsWith("![") && t.endsWith(")") && !t.startsWith("![[")) {
            val i = t.indexOf("](")
            if (i > 0 && t.indexOf(')', i) == t.length - 1) {
                val raw = t.substring(i + 2, t.length - 1).trim().removePrefix("<").removeSuffix(">")
                if (!raw.contains("://")) return raw.replace("%20", " ")
            }
        }
        return null
    }

    /**
     * Découpe une note : les images et whiteboards seuls sur leur ligne deviennent des blocs à part (affichés à la
     * bonne taille par l'appli, comme dans l'éditeur de bureau) ; `resolve` renvoie le chemin du fichier.
     */
    fun blocks(src: String, resolve: (String) -> Embed?): List<Block> {
        val out = mutableListOf<Block>()
        val buf = StringBuilder()
        var fence = false
        fun flush() {
            if (buf.isNotBlank()) out += Block.Text(buf.toString())
            buf.clear()
        }
        for (line in src.split('\n')) {
            val t = line.trimStart()
            if (t.startsWith("```") || t.startsWith("~~~")) fence = !fence
            val target = if (fence) null else embedTarget(line)
            when (val e = target?.let(resolve)) {
                is Embed.Image -> { flush(); out += Block.Image(e.uri) }
                is Embed.Board -> { flush(); out += Block.Board(e.path) }
                null -> buf.append(line).append('\n')
            }
        }
        flush()
        return out
    }

    /** Liens `[[…]]` d'une note (cibles, sans doublon). */
    fun links(content: String): List<String> {
        val out = linkedSetOf<String>()
        var i = content.indexOf("[[")
        while (i >= 0) {
            val end = content.indexOf("]]", i + 2)
            if (end < 0) break
            val inner = content.substring(i + 2, end)
            if (!inner.contains('\n') && !inner.contains(']')) {
                val t = inner.substringBefore('|').substringBefore('#').trim()
                if (t.isNotEmpty()) out += t
            }
            i = content.indexOf("[[", end + 2)
        }
        return out.toList()
    }
}
