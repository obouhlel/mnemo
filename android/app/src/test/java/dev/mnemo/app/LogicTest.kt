package dev.mnemo.app

import dev.mnemo.app.data.Element
import dev.mnemo.app.data.Pt
import dev.mnemo.app.data.Sketch
import dev.mnemo.app.data.fileHash
import dev.mnemo.app.data.parseDbUrl
import dev.mnemo.app.data.simplify
import dev.mnemo.app.markdown.Preprocess
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Logique partagée avec Mnemo sur ordinateur : mêmes résultats que les tests Rust. */
class LogicTest {
    @Test
    fun fileHashMatchesDesktop() {
        // valeurs calculées par file_hash (src/db/vault_sync.rs)
        assertEquals("cbf29ce484222325", fileHash(ByteArray(0)))
        assertEquals("a430d84680aabd0b", fileHash("hello".toByteArray()))
    }

    @Test
    fun databaseUrls() {
        val c = parseDbUrl("postgres://bob:p%40ss@db.example.com:5432/vault?sslmode=require")!!
        assertEquals("bob", c.user)
        assertEquals("p@ss", c.password)
        assertTrue(c.ssl)
        assertTrue(c.jdbcUrl.startsWith("jdbc:postgresql://db.example.com:5432/vault?ssl=true"))
        val local = parseDbUrl("postgresql://postgres:postgres@localhost/vault")!!
        assertFalse(local.ssl)
        assertTrue(local.jdbcUrl.contains("sslmode=disable"))
        assertEquals("postgres://postgres@localhost/vault", local.display)
        assertNull(parseDbUrl("mysql://x"))
        assertNull(parseDbUrl("postgres://"))
    }

    @Test
    fun markdownLikeDesktop() {
        assertEquals("voir [Ma Note](mnemo://note/Ma%20Note) ici", Preprocess.process("voir [[Ma Note]] ici"))
        assertEquals("[alias](mnemo://note/a%2Fb)", Preprocess.process("[[a/b#Titre|alias]]"))
        assertEquals("`[[pas un lien]]`", Preprocess.process("`[[pas un lien]]`"))
        assertEquals("```\n[[x]]\n```\n[y](mnemo://note/y)", Preprocess.process("```\n[[x]]\n```\n[[y]]"))
        assertEquals("# Titre\n[#projet/rust](mnemo://tag/projet%2Frust) ok", Preprocess.process("# Titre\n#projet/rust ok"))
        assertEquals("issue#12 et #42", Preprocess.process("issue#12 et #42"))
        val resolve = { t: String ->
            when (t) {
                "photo.png", "img/photo.png" -> Preprocess.Embed.Image("file:///v/img/photo.png")
                "croquis.mnemo-board" -> Preprocess.Embed.Board("Dessins/croquis.mnemo-board")
                else -> null
            }
        }
        assertEquals("a ![photo.png](file:///v/img/photo.png) b", Preprocess.process("a ![[photo.png]] b", resolve))
        assertEquals("![photo.png](file:///v/img/photo.png)", Preprocess.process("![[photo.png|300]]", resolve))
        assertEquals("![légende](file:///v/img/photo.png)", Preprocess.process("![légende](img/photo.png)", resolve))
        assertEquals("![x](https://exemple.fr/a.png)", Preprocess.process("![x](https://exemple.fr/a.png)", resolve))
        assertEquals("[🖊 croquis.mnemo-board](mnemo://board/Dessins%2Fcroquis.mnemo-board)", Preprocess.process("![[croquis.mnemo-board]]", resolve))
        assertEquals(listOf("B", "c", "Dir/D"), Preprocess.links("a [[B]] [[c|x]] [[Dir/D#t]] [[B]] [[ ]]"))
    }

    @Test
    fun embedBlocks() {
        val resolve = { t: String ->
            when (t) {
                "a.png" -> Preprocess.Embed.Image("Img/a.png")
                "d.mnemo-board" -> Preprocess.Embed.Board("d.mnemo-board")
                else -> null
            }
        }
        val b = Preprocess.blocks("# T\ntexte\n![[a.png|200]]\nsuite ![[a.png]] en ligne\n```\n![[a.png]]\n```\n![[d.mnemo-board]]\n![[absent.png]]", resolve)
        assertEquals(5, b.size)
        assertEquals(Preprocess.Block.Image("Img/a.png"), b[1])
        assertTrue((b[2] as Preprocess.Block.Text).markdown.contains("en ligne")) // pas seule sur sa ligne : reste dans le texte
        assertTrue((b[2] as Preprocess.Block.Text).markdown.contains("```\n![[a.png]]\n```")) // dans un bloc de code
        assertEquals(Preprocess.Block.Board("d.mnemo-board"), b[3])
        assertEquals("![]( x y.png )".let { Preprocess.embedTarget("![](x%20y.png)") }, "x y.png")
    }

    @Test
    fun sketchReadsDesktopFormat() {
        // fichier tel qu'écrit par Mnemo sur ordinateur (serde_json, to_string_pretty)
        val desktop = """
            { "version": 1, "elements": [
              { "type": "stroke", "color": "#1e1e1e", "width": 2.0, "points": [[0.0, 0.0], [10.0, 4.0]] },
              { "type": "rect", "color": "#1971c2", "width": 4.0, "x": 10.0, "y": 10.0, "w": -10.0, "h": 20.0 },
              { "type": "arrow", "color": "#000000", "width": 2.0, "points": [[0.0, 0.0], [50.0, 50.0]] },
              { "type": "text", "color": "#e03131", "size": 20.0, "x": 1.0, "y": 2.0, "text": "Été" },
              { "type": "futur", "color": "#000000", "x": 1 }
            ] }
        """.trimIndent()
        val s = Sketch.parse(desktop)
        assertEquals(5, s.elements.size)
        assertTrue(s.elements[0] is Element.Stroke)
        assertEquals(listOf(0f, 10f, 10f, 30f), s.elements[1].bounds().toList())
        assertTrue((s.elements[2] as Element.Line).arrow)
        assertEquals("Été", (s.elements[3] as Element.Text).text)
        assertTrue(s.elements[4] is Element.Unknown) // gardé tel quel
        val back = Sketch.parse(s.toJson())
        assertEquals(s.elements.take(4), back.elements.take(4))
        assertTrue(s.toJson().contains("\"type\": \"futur\""))
        assertEquals(3, s.hit(Pt(5f, 20f), 1f)) // le texte, dessiné par-dessus le rectangle
        assertEquals(1, s.hit(Pt(5f, 29f), 1f))
    }

    @Test
    fun strokeSimplification() {
        val line = (0..100).map { Pt(it.toFloat(), 0f) }
        assertEquals(listOf(Pt(0f, 0f), Pt(100f, 0f)), simplify(line, 0.5f))
        assertEquals(3, simplify(listOf(Pt(0f, 0f), Pt(5f, 0.1f), Pt(10f, 0f), Pt(10f, 10f)), 0.5f).size)
    }

    @Test
    fun searchSnippets() {
        val c = "# Plan\n\n![[schema.png]]\n\nÉtapes suivantes : *Android*."
        val s = dev.mnemo.app.ui.searchSnippet(c, c.lowercase().indexOf("android"))
        org.junit.Assert.assertEquals("… ![[schema.png]]  Étapes suivantes : *Android*.", s)
        val long = "mot ".repeat(30) + "cible " + "fin ".repeat(40)
        val t = dev.mnemo.app.ui.searchSnippet(long, long.indexOf("cible"))
        org.junit.Assert.assertTrue(t, t.startsWith("… mot") && t.endsWith(" …") && "cible" in t)
    }
}
