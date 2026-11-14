package dev.mnemo.app

import androidx.test.core.app.ApplicationProvider
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextInput
import dev.mnemo.app.data.LocalDb
import dev.mnemo.app.data.Sketch
import dev.mnemo.app.data.SyncEngine
import dev.mnemo.app.data.fileHash
import dev.mnemo.app.data.parseDbUrl
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import java.io.File
import java.sql.Connection
import java.sql.DriverManager
import java.util.UUID

/**
 * Synchronisation réelle avec PostgreSQL, schéma de Mnemo sur ordinateur (../src/schema.sql).
 * Base : MNEMO_TEST_PG (par défaut postgres://postgres:postgres@localhost:5432). Ignoré si elle est injoignable.
 */
@RunWith(RobolectricTestRunner::class)
class SyncTest {
    @get:org.junit.Rule
    val compose = androidx.compose.ui.test.junit4.createComposeRule()

    private val server = System.getenv("MNEMO_TEST_PG") ?: "postgres://postgres:postgres@localhost:5432"
    private val dbName = "mnemo_android_test"
    private lateinit var cfg: dev.mnemo.app.data.DbConfig
    private lateinit var local: LocalDb
    private lateinit var engine: SyncEngine

    private fun admin(db: String): Connection {
        val c = parseDbUrl("$server/$db")!!
        return DriverManager.getConnection(c.jdbcUrl, c.user, c.password)
    }

    private fun Connection.exec(sql: String, vararg args: Any?) = prepareStatement(sql).use { st ->
        args.forEachIndexed { i, a -> st.setObject(i + 1, a) }
        st.execute()
    }

    private fun Connection.one(sql: String, vararg args: Any?): Any? = prepareStatement(sql).use { st ->
        args.forEachIndexed { i, a -> st.setObject(i + 1, a) }
        st.executeQuery().use { rs -> if (rs.next()) rs.getObject(1) else null }
    }

    @Before
    fun setUp() {
        Class.forName("org.postgresql.Driver")
        val reachable = runCatching { admin("postgres").use { true } }.getOrDefault(false)
        assumeTrue("PostgreSQL injoignable ($server) : test ignoré", reachable)
        admin("postgres").use { c ->
            c.exec("DROP DATABASE IF EXISTS $dbName")
            c.exec("CREATE DATABASE $dbName")
        }
        // schéma créé comme le fait Mnemo sur ordinateur, puis une note et une image « venues du PC »
        admin(dbName).use { c ->
            c.createStatement().use { it.execute(File("../../src/schema.sql").readText()) }
            c.exec("INSERT INTO notes (path, content) VALUES ('Projets/Plan', 'Voir [[Accueil]]'), ('Accueil', '# Accueil')")
            val png = byteArrayOf(1, 2, 3, 4, 5)
            c.exec("INSERT INTO files (path, data, hash, size) VALUES ('Images/a.png', ?, ?, 5)", png, fileHash(png))
            c.exec("INSERT INTO projects (name, github_repo) VALUES ('Mnemo', 'obouhlel/mnemo')")
            c.exec("INSERT INTO tickets (project_id, title, status) SELECT id, 'Appli Android', 'doing' FROM projects")
        }
        cfg = parseDbUrl("$server/$dbName")!!
        local = LocalDb(ApplicationProvider.getApplicationContext(), "test-${UUID.randomUUID()}.db")
        engine = SyncEngine(local, "android-test")
    }

    @Test
    fun fullCycle() {
        assertNull(engine.test(cfg))

        // 1. première synchro : tout arrive sur l'appareil
        engine.sync(cfg)
        assertEquals(listOf("Accueil", "Projets/Plan"), local.notes().map { it.path })
        assertEquals(listOf("Images/a.png"), local.files().map { it.path })
        assertEquals(listOf<Byte>(1, 2, 3, 4, 5), local.fileOnDisk("Images/a.png").readBytes().toList())
        assertTrue(local.meta("board")!!.contains("Appli Android"))

        admin(dbName).use { pc ->
            // 2. modification sur l'appareil → envoyée, marquée comme venant du téléphone
            val plan = local.noteByPath("Projets/Plan")!!
            local.saveLocal(plan.id, plan.path, "Modifié sur le téléphone")
            assertEquals(1, local.pendingCount())
            engine.sync(cfg)
            assertEquals("Modifié sur le téléphone", pc.one("SELECT content FROM notes WHERE id = ?::uuid", plan.id))
            assertEquals("android-test", pc.one("SELECT updated_by FROM notes WHERE id = ?::uuid", plan.id))
            assertEquals(0, local.pendingCount())

            // 3. modification sur le PC → reçue
            pc.exec("UPDATE notes SET content = 'Modifié sur le PC' WHERE id = ?::uuid", plan.id)
            engine.sync(cfg)
            assertEquals("Modifié sur le PC", local.note(plan.id)!!.content)

            // 4. conflit : modifiée des deux côtés → la base gagne, le texte du téléphone est gardé à part
            local.saveLocal(plan.id, plan.path, "Version téléphone")
            pc.exec("UPDATE notes SET content = 'Version PC' WHERE id = ?::uuid", plan.id)
            engine.sync(cfg)
            engine.sync(cfg)
            assertEquals("Version PC", local.note(plan.id)!!.content)
            val copy = local.notes().single { it.path.startsWith("Projets/Plan (conflit mobile") }
            assertEquals("Version téléphone", copy.content)

            // 5. note créée sur le téléphone : même identifiant dans la base
            val id = UUID.randomUUID().toString()
            local.saveLocal(id, "Mobile/Idée", "Créée dans le métro")
            engine.sync(cfg)
            assertEquals("Mobile/Idée", pc.one("SELECT path FROM notes WHERE id = ?::uuid", id))

            // 6. suppression sur le téléphone
            local.markDeleted(id)
            engine.sync(cfg)
            assertNull(pc.one("SELECT path FROM notes WHERE id = ?::uuid", id))

            // 7. suppression sur le PC
            pc.exec("DELETE FROM notes WHERE path = 'Accueil'")
            engine.sync(cfg)
            assertNull(local.noteByPath("Accueil"))

            // 8. whiteboard dessiné sur la tablette → fichier dans la base, même empreinte que le PC calculerait
            val json = Sketch().toJson().toByteArray()
            local.writeLocalFile("Dessins/tablette.mnemo-board", json)
            engine.sync(cfg)
            assertEquals(fileHash(json), pc.one("SELECT hash FROM files WHERE path = 'Dessins/tablette.mnemo-board'"))

            // 9. image modifiée sur le PC → retéléchargée
            val png2 = byteArrayOf(9, 9, 9)
            pc.exec("UPDATE files SET data = ?, hash = ?, size = 3 WHERE path = 'Images/a.png'", png2, fileHash(png2))
            engine.sync(cfg)
            assertEquals(listOf<Byte>(9, 9, 9), local.fileOnDisk("Images/a.png").readBytes().toList())
            assertNotNull(local.meta("last_sync"))
        }
    }

    /** Parcours du premier lancement : saisir l'adresse, Connecter, les notes de la base s'affichent. */
    @Test
    fun firstRunConnect() {
        val app = ApplicationProvider.getApplicationContext<android.app.Application>()
        app.getSharedPreferences("mnemo", 0).edit().clear().apply()
        val model = dev.mnemo.app.ui.AppModel(app, "ui-${UUID.randomUUID()}.db")
        compose.setContent { dev.mnemo.app.ui.MnemoTheme { dev.mnemo.app.ui.MnemoApp(model) } }
        compose.onNodeWithText("Afficher l'adresse").performScrollTo().performClick()
        compose.onNodeWithText("Masquer l'adresse").assertExists()
        compose.onNode(androidx.compose.ui.test.hasSetTextAction()).performTextInput("$server/$dbName")
        compose.onNodeWithText("Connecter").performScrollTo().performClick()
        compose.waitUntil(20_000) { compose.onAllNodesWithText("Projets").fetchSemanticsNodes().isNotEmpty() }
        assertEquals("$server/$dbName", model.dbUrl())
    }
}
