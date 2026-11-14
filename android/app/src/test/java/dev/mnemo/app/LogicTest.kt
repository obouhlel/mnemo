package dev.mnemo.app

import dev.mnemo.app.data.Element
import dev.mnemo.app.data.Pt
import dev.mnemo.app.data.Sketch
import dev.mnemo.app.data.DbIdentity
import dev.mnemo.app.data.conflictName
import dev.mnemo.app.data.fileConflictBase
import dev.mnemo.app.data.fileHash
import dev.mnemo.app.data.firstFree
import dev.mnemo.app.data.isSafeNotePath
import dev.mnemo.app.data.isSafeRelPath
import dev.mnemo.app.data.newNotePath
import dev.mnemo.app.data.normalizePath
import dev.mnemo.app.data.percentDecode
import dev.mnemo.app.data.portableName
import dev.mnemo.app.data.parseDbUrl
import dev.mnemo.app.data.simplify
import dev.mnemo.app.markdown.Preprocess
import dev.mnemo.app.ui.sampleSize
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
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
        assertTrue(c.jdbcUrl.startsWith("jdbc:postgresql://db.example.com:5432/vault?sslmode=require&"))
        val local = parseDbUrl("postgresql://postgres:postgres@localhost/vault")!!
        assertFalse(local.ssl)
        assertTrue(local.jdbcUrl.contains("sslmode=disable"))
        assertEquals("postgres://postgres@localhost/vault", local.display)
        assertNull(parseDbUrl("mysql://x"))
        assertNull(parseDbUrl("postgres://"))
    }

    /** `+` dans un mot de passe reste `+` (URLDecoder en faisait une espace). */
    @Test
    fun passwordWithPlusAndPercent() {
        val c = parseDbUrl("postgres://a+b:p+w%2B%20%C3%A9%25@h/db")!!
        assertEquals("a+b", c.user)
        assertEquals("p+w+ é%", c.password)
        assertEquals("100%", percentDecode("100%"))
        assertEquals("%zz+", percentDecode("%zz+"))
        assertEquals("a@b", parseDbUrl("postgres://u:a%40b@h/db")!!.password)
    }

    /** Seul `require` (et moins) se passe de vérifier le certificat ; `verify-*` utilise les autorités du système. */
    @Test
    fun sslModes() {
        fun jdbc(mode: String) = parseDbUrl("postgres://u:p@h/db?sslmode=$mode")?.jdbcUrl.orEmpty()
        assertTrue(jdbc("require").contains("sslmode=require&sslfactory=org.postgresql.ssl.NonValidatingFactory"))
        for (mode in listOf("verify-ca", "verify-full")) {
            assertTrue(jdbc(mode).contains("sslmode=$mode&sslfactory=org.postgresql.ssl.DefaultJavaSSLFactory"))
            assertFalse(jdbc(mode).contains("NonValidating"))
        }
        assertTrue(jdbc("disable").contains("sslmode=disable"))
        assertFalse(parseDbUrl("postgres://u:p@h/db?sslmode=disable")!!.ssl)
        assertTrue(parseDbUrl("postgres://u:p@h/db?sslmode=verify-full")!!.ssl)
        assertNull(parseDbUrl("postgres://u:p@h/db?sslmode=verify_full")) // faute de frappe : pas de repli silencieux
        // même base sous une adresse réécrite
        assertEquals(parseDbUrl("postgres://u:p@H/db")!!.identity, parseDbUrl("postgresql://autre:x@h:5432/db?sslmode=require")!!.identity)
        assertTrue(DbIdentity("h:5432/db", "A").sameAs(DbIdentity("autre:5432/db", "A")))
        assertFalse(DbIdentity("h:5432/db", "A").sameAs(DbIdentity("h:5432/db", "B")))
        assertTrue(DbIdentity("h:5432/db", null).sameAs(DbIdentity("h:5432/db", "B"))) // identifiant apparu depuis
        assertFalse(DbIdentity("h:5432/db", null).sameAs(DbIdentity("h:5432/autre", null)))
    }

    /** Mêmes règles que `normalize_path` et `portable_name` (src/model.rs). */
    @Test
    fun notePathsLikeDesktop() {
        assertEquals("a/b/c", normalizePath(" a// b /c.md ").getOrThrow())
        assertEquals("a/b", normalizePath("/a/b.MD").getOrThrow())
        for (bad in listOf("  / ", "", "x[y]", "a|b", "a#b", "a^b", "a\\b", "a/../b", "./a", "a/.caché", "a\tb", "x".repeat(501), "é".repeat(126))) {
            assertTrue(bad, normalizePath(bad).isFailure)
        }
        assertTrue(normalizePath("é".repeat(125)).isSuccess) // 250 octets
        assertEquals("Le nom ne peut pas être vide", normalizePath(".md").exceptionOrNull()!!.message)
        assertNull(portableName("Projets/Plan (v2)"))
        for (bad in listOf("a:b", "a*b", "a?b", "a\"b", "a<b", "a>b", "fin./x", "CON", "con.txt", "x/Com1", "LPT9.md", "nul")) {
            assertNotNull(bad, portableName(bad))
        }
        for (ok in listOf("COM0", "COM10", "Console", "LPT", "a.b")) assertNull(ok, portableName(ok))
        assertEquals("Réunion", newNotePath(" Réunion.md ").getOrThrow())
        assertTrue(newNotePath("Réunion : plan").isFailure)
        assertTrue(newNotePath("a/ ../b").isFailure)
    }

    @Test
    fun pathsFromTheDatabase() {
        for (ok in listOf("a.png", "Pièces jointes/a b.png", "a/b/c.mnemo-board")) assertTrue(ok, isSafeRelPath(ok))
        for (bad in listOf("", "../a", "a/../b", "/a", "a/", "a//b", "a\\b", "c:a", "C:/a", ".git/config", "a/.caché", "a/ /b", "a\u0000b")) {
            assertFalse(bad, isSafeRelPath(bad))
        }
        assertTrue(isSafeNotePath("Réunion : plan"))
        assertFalse(isSafeNotePath("../Réunion"))
    }

    @Test
    fun conflictNames() {
        val at = java.util.Date(0)
        val name = conflictName("Dossier/Note", at)
        assertTrue(name.matches(Regex("Dossier/Note \\(conflit mobile \\d{4}-\\d{2}-\\d{2} \\d{6}\\)"))) // à la seconde
        assertTrue(name != conflictName("Dossier/Note", java.util.Date(1000)))
        assertNull(portableName(name))
        val taken = mutableSetOf<String>()
        val names = (1..3).map { firstFree(name) { it in taken }.also { n -> taken += n } }
        assertEquals(listOf(name, "$name 2", "$name 3"), names)
        assertEquals("a/d (conflit mobile)" to ".mnemo-board", fileConflictBase("a/d.mnemo-board"))
        assertEquals("a.b/sans (conflit mobile)" to "", fileConflictBase("a.b/sans"))
        assertEquals("d (conflit mobile) 2.png", firstFree("d (conflit mobile)", ".png") { it == "d (conflit mobile).png" })
    }

    /** Une intégration sans fichier affichable ne devient jamais un lien qui crée une note. */
    @Test
    fun unresolvedEmbeds() {
        assertEquals("[doc.pdf](mnemo://embed/doc.pdf)", Preprocess.process("![[doc.pdf]]"))
        assertEquals("[absente.png](mnemo://embed/absente.png)", Preprocess.process("![[absente.png|300]]"))
        assertEquals("[Note](mnemo://note/Note)", Preprocess.process("[[Note]]"))
        assertTrue(Preprocess.isImage("Schémas/plan.SVG"))
        assertFalse(Preprocess.isImage("doc.pdf"))
    }

    @Test
    fun imagesAreDecodedAtABoundedSize() {
        assertEquals(1, sampleSize(1920, 1080, 2048))
        assertEquals(2, sampleSize(4000, 3000, 2048))
        assertEquals(8, sampleSize(12000, 9000, 2048))
        assertEquals(1, sampleSize(0, 0, 2048))
    }

    /** Un whiteboard illisible n'est pas un whiteboard vide. */
    @Test
    fun unreadableSketchIsNotEmpty() {
        assertTrue(Sketch.parse("").elements.isEmpty())
        assertTrue(Sketch.parse("{\"version\": 1, \"elements\": []}").elements.isEmpty())
        for (bad in listOf("{\"version\": 1, \"elem", "pas du json", "{}", "[]", "{\"elements\": 3}")) {
            assertTrue(bad, runCatching { Sketch.parse(bad) }.isFailure)
        }
        assertEquals(fileHash("hello".toByteArray()), fileHash("hello".byteInputStream()))
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
}
