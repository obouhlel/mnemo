package dev.mnemo.app

import androidx.test.core.app.ApplicationProvider
import dev.mnemo.app.data.LocalDb
import dev.mnemo.app.data.Sketch
import dev.mnemo.app.data.SyncEngine
import dev.mnemo.app.data.fileHash
import dev.mnemo.app.data.parseDbUrl
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
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
 * Cas où la synchronisation pouvait perdre des données ou se bloquer (docs/AUDIT.md, § 4), contre un vrai
 * PostgreSQL avec le schéma du bureau. Base : MNEMO_TEST_PG, comme SyncTest. Ignoré si elle est injoignable.
 */
@RunWith(RobolectricTestRunner::class)
class SyncSafetyTest {
    private val server = System.getenv("MNEMO_TEST_PG") ?: "postgres://postgres:postgres@localhost:5432"
    private val dbName = "mnemo_android_safety"
    private lateinit var cfg: dev.mnemo.app.data.DbConfig
    private lateinit var local: LocalDb
    private lateinit var engine: SyncEngine
    private lateinit var pc: Connection

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

    private fun count(sql: String, vararg args: Any?): Int = (pc.one(sql, *args) as Number).toInt()

    @Before
    fun setUp() {
        Class.forName("org.postgresql.Driver")
        val reachable = runCatching { admin("postgres").use { true } }.getOrDefault(false)
        assumeTrue("PostgreSQL injoignable ($server) : test ignoré", reachable)
        admin("postgres").use { c ->
            c.exec("DROP DATABASE IF EXISTS $dbName")
            c.exec("CREATE DATABASE $dbName")
        }
        pc = admin(dbName)
        pc.createStatement().use { it.execute(File("../../src/schema.sql").readText()) }
        pc.exec("INSERT INTO notes (path, content) VALUES ('Plan', 'v1'), ('Accueil', '# Accueil')")
        cfg = parseDbUrl("$server/$dbName")!!
        local = LocalDb(ApplicationProvider.getApplicationContext(), "safety-${UUID.randomUUID()}.db")
        local.vaultDir.deleteRecursively()
        local.vaultDir.mkdirs()
        engine = SyncEngine(local, "android-test")
        engine.sync(cfg)
    }

    @org.junit.After
    fun tearDown() {
        if (::pc.isInitialized) pc.close()
    }

    /** Coupure avant le COMMIT : rien n'est noté « envoyé », tout repart à la synchronisation suivante. */
    @Test
    fun interruptedBeforeCommit() {
        val plan = local.noteByPath("Plan")!!
        local.saveEdit(plan.id, plan.path, "v1", "téléphone")
        val fresh = UUID.randomUUID().toString()
        local.saveLocal(fresh, "Idée", "dans le métro")
        local.markDeleted(local.noteByPath("Accueil")!!.id)
        local.writeLocalFile("Dessins/a.mnemo-board", Sketch().toJson().toByteArray())
        pc.exec("INSERT INTO notes (path, content) VALUES ('Du PC', 'x')")

        val cut = runCatching { engine.sync(cfg) { throw java.io.IOException("réseau coupé") } }
        assertTrue(cut.isFailure)
        assertEquals(4, local.pendingCount())
        assertEquals("téléphone", local.note(plan.id)!!.content)
        assertTrue(local.note(plan.id)!!.dirty)
        assertEquals(0, local.note(fresh)!!.version)
        assertNull(local.noteByPath("Du PC")) // rien de la base n'est appliqué non plus
        assertEquals("v1", pc.one("SELECT content FROM notes WHERE path = 'Plan'"))
        assertEquals(0, count("SELECT count(*) FROM files"))
        assertEquals(1, count("SELECT count(*) FROM notes WHERE path = 'Accueil'"))

        val r = engine.sync(cfg)
        assertEquals(0, r.conflicts)
        assertEquals(0, local.pendingCount())
        assertEquals("téléphone", pc.one("SELECT content FROM notes WHERE path = 'Plan'"))
        assertEquals("dans le métro", pc.one("SELECT content FROM notes WHERE id = ?::uuid", fresh))
        assertEquals(0, count("SELECT count(*) FROM notes WHERE path = 'Accueil'"))
        assertEquals(1, count("SELECT count(*) FROM files"))
        assertNotNull(local.noteByPath("Du PC"))
    }

    /** Coupure juste après le COMMIT (la copie locale n'a pas été mise à jour) : pas de faux conflit ensuite. */
    @Test
    fun interruptedAfterCommit() {
        val plan = local.noteByPath("Plan")!!
        local.saveEdit(plan.id, plan.path, "v1", "téléphone")
        val fresh = UUID.randomUUID().toString()
        local.saveLocal(fresh, "Idée", "dans le métro")
        engine.sync(cfg)
        // l'appareil revient à l'état d'avant : toujours « à envoyer », avec les anciennes versions
        local.putRemote(plan.id, plan.path, "v1", plan.version, 0)
        local.saveEdit(plan.id, plan.path, "v1", "téléphone")
        local.removeIfClean(fresh)
        local.saveLocal(fresh, "Idée", "dans le métro")

        val r = engine.sync(cfg)
        assertEquals(0, r.conflicts)
        assertEquals(0, local.pendingCount())
        assertEquals(3, count("SELECT count(*) FROM notes"))
        assertEquals(pc.one("SELECT version FROM notes WHERE path = 'Plan'"), local.note(plan.id)!!.version)
    }

    /** Taper puis effacer : le déclencheur de la base ignore la mise à jour, ce n'est pas un conflit. */
    @Test
    fun noOpEditIsNotAConflict() {
        val plan = local.noteByPath("Plan")!!
        local.saveEdit(plan.id, plan.path, "v1", "v1 x")
        local.saveEdit(plan.id, plan.path, "v1 x", "v1")
        assertTrue(local.note(plan.id)!!.dirty)
        val r = engine.sync(cfg)
        assertEquals(0, r.conflicts)
        assertEquals(0, local.pendingCount())
        assertEquals(2, count("SELECT count(*) FROM notes"))
        assertEquals(plan.version, local.note(plan.id)!!.version)
    }

    /** Le bureau modifie la note pendant que le téléphone a une saisie non enregistrée : rien n'est écrasé. */
    @Test
    fun desktopEditPulledWhileTyping() {
        val plan = local.noteByPath("Plan")!!
        pc.exec("UPDATE notes SET content = 'bureau' WHERE path = 'Plan'")
        engine.sync(cfg) // reçue : la note n'était pas encore modifiée dans la copie locale
        assertEquals("bureau", local.note(plan.id)!!.content)
        // l'éditeur enregistre sa saisie, faite sur « v1 »
        val target = local.saveEdit(plan.id, plan.path, "v1", "v1 + téléphone")
        assertTrue(target != plan.id)
        assertEquals("bureau", local.note(plan.id)!!.content)
        assertFalse(local.note(plan.id)!!.dirty)
        assertTrue(local.note(target)!!.path.startsWith("Plan (conflit mobile "))
        engine.sync(cfg)
        assertEquals("bureau", pc.one("SELECT content FROM notes WHERE path = 'Plan'"))
        assertEquals("v1 + téléphone", pc.one("SELECT content FROM notes WHERE id = ?::uuid", target))
    }

    /** Une note enregistrée pendant l'envoi reste « à envoyer », et son envoi suivant n'est pas un conflit. */
    @Test
    fun savedDuringPush() {
        val plan = local.noteByPath("Plan")!!
        local.saveEdit(plan.id, plan.path, "v1", "premier jet")
        local.writeLocalFile("d.mnemo-board", "{\"version\":1,\"elements\":[]}".toByteArray())
        val later = "{\"version\": 1, \"elements\": []}".toByteArray()
        engine.sync(cfg) {
            local.saveEdit(plan.id, plan.path, "premier jet", "second jet")
            local.writeLocalFile("d.mnemo-board", later)
        }
        assertEquals("premier jet", pc.one("SELECT content FROM notes WHERE path = 'Plan'"))
        assertEquals("second jet", local.note(plan.id)!!.content)
        assertTrue(local.note(plan.id)!!.dirty)
        assertTrue(local.fileRow("d.mnemo-board")!!.dirty)
        val r = engine.sync(cfg)
        assertEquals(0, r.conflicts)
        assertEquals("second jet", pc.one("SELECT content FROM notes WHERE path = 'Plan'"))
        assertEquals(fileHash(later), pc.one("SELECT hash FROM files WHERE path = 'd.mnemo-board'"))
        assertEquals(0, count("SELECT count(*) FROM files WHERE path LIKE '%conflit%'"))
        assertEquals(0, local.pendingCount())
    }

    /** Une note reçue pendant qu'elle est modifiée ici ne remplace pas la saisie. */
    @Test
    fun pullDoesNotClobberEditMadeDuringSync() {
        val plan = local.noteByPath("Plan")!!
        pc.exec("UPDATE notes SET content = 'bureau' WHERE path = 'Plan'")
        engine.sync(cfg) { local.saveEdit(plan.id, plan.path, "v1", "téléphone") }
        assertEquals("téléphone", local.note(plan.id)!!.content)
        val r = engine.sync(cfg)
        assertEquals(1, r.conflicts)
        assertEquals("bureau", local.note(plan.id)!!.content)
        assertEquals("téléphone", local.notes().single { it.path.startsWith("Plan (conflit mobile") }.content)
    }

    /** Deux conflits coup sur coup sur la même note : deux copies, aucune perdue. */
    @Test
    fun twoConflictsInTheSameMinute() {
        val plan = local.noteByPath("Plan")!!
        for (i in 1..2) {
            local.saveEdit(plan.id, plan.path, null, "téléphone $i")
            pc.exec("UPDATE notes SET content = ? WHERE path = 'Plan'", "bureau $i")
            assertEquals(1, engine.sync(cfg).conflicts)
            assertEquals("bureau $i", local.note(plan.id)!!.content)
        }
        val copies = local.notes().filter { it.path.startsWith("Plan (conflit mobile") }.map { it.content }.sorted()
        assertEquals(listOf("téléphone 1", "téléphone 2"), copies)
        assertEquals(2, count("SELECT count(*) FROM notes WHERE path LIKE 'Plan (conflit mobile%'"))
    }

    /** Un nom déjà pris (ou refusé par la base) ne bloque pas la synchronisation : la note prend un nom « (mobile) ». */
    @Test
    fun takenOrInvalidNameDoesNotBlockSync() {
        val plan = local.noteByPath("Plan")!!
        local.rename(plan.id, "accueil") // même nom qu'« Accueil », à la casse près
        val twin = UUID.randomUUID().toString()
        local.saveLocal(twin, "ACCUEIL", "homonyme créé hors ligne")
        val empty = UUID.randomUUID().toString()
        local.saveLocal(empty, "", "sans nom")
        val other = UUID.randomUUID().toString()
        local.saveLocal(other, "Autre", "ok")

        engine.sync(cfg)
        assertEquals(0, local.pendingCount())
        assertEquals("accueil (mobile)", local.note(plan.id)!!.path)
        assertEquals("accueil (mobile)", pc.one("SELECT path FROM notes WHERE id = ?::uuid", plan.id))
        assertEquals("ACCUEIL (mobile 2)", pc.one("SELECT path FROM notes WHERE id = ?::uuid", twin))
        assertEquals("Sans titre (mobile)", local.note(empty)!!.path)
        assertEquals("ok", pc.one("SELECT content FROM notes WHERE id = ?::uuid", other))
        engine.sync(cfg) // et la suivante passe aussi
    }

    /** Modifiée ici, supprimée sur le bureau : la modification n'est pas perdue. */
    @Test
    fun editedHereDeletedThere() {
        val plan = local.noteByPath("Plan")!!
        local.saveEdit(plan.id, plan.path, "v1", "téléphone")
        pc.exec("DELETE FROM notes WHERE path = 'Plan'")
        engine.sync(cfg)
        assertEquals("téléphone", pc.one("SELECT content FROM notes WHERE id = ?::uuid", plan.id))
        assertEquals(0, local.pendingCount())
    }

    /** Un chemin `../…` venu de la base n'écrit rien hors du coffre ; les notes au chemin invalide sont ignorées. */
    @Test
    fun hostilePathsStayOutside() {
        val data = byteArrayOf(1, 2, 3)
        // lignes d'avant les contraintes « *_path_safe » (posées NOT VALID : les anciennes lignes restent), ou écrites à la main
        pc.exec("ALTER TABLE files DROP CONSTRAINT IF EXISTS files_path_safe")
        pc.exec("ALTER TABLE notes DROP CONSTRAINT IF EXISTS notes_path_safe")
        for (p in listOf("../evil.png", "a/../../evil.png", ".cache/x.png", "c:evil.png", "a\\b.png")) {
            pc.exec("INSERT INTO files (path, data, hash, size) VALUES (?, ?, ?, 3)", p, data, fileHash(data))
        }
        pc.exec("INSERT INTO files (path, data, hash, size) VALUES ('ok/a.png', ?, ?, 3)", data, fileHash(data))
        pc.exec("INSERT INTO notes (path, content) VALUES ('../dehors', 'x'), ('Réunion : plan', 'ok')")
        engine.sync(cfg)
        assertEquals(listOf("ok/a.png"), local.files().map { it.path })
        assertFalse(File(local.vaultDir.parentFile, "evil.png").exists())
        assertFalse(File(local.vaultDir.parentFile!!.parentFile, "evil.png").exists())
        assertNull(local.noteByPath("../dehors"))
        assertNotNull(local.noteByPath("Réunion : plan"))
        assertEquals(0, engine.sync(cfg).pulled) // les lignes ignorées ne sont pas « reçues » à chaque passage
        assertEquals(6, count("SELECT count(*) FROM files")) // et rien n'est supprimé dans la base
    }

    /** Pièce jointe de plusieurs morceaux (plus de 1 Mo) : reçue entière, puis renvoyée modifiée. */
    @Test
    fun largeFileRoundTrip() {
        val big = ByteArray(2_500_000) { (it * 31 % 251).toByte() }
        pc.exec("INSERT INTO files (path, data, hash, size) VALUES ('gros.bin', ?, ?, ?)", big, fileHash(big), big.size.toLong())
        engine.sync(cfg)
        assertEquals(fileHash(big), fileHash(local.fileOnDisk("gros.bin").readBytes()))
        assertEquals(fileHash(big), local.fileRow("gros.bin")!!.localHash)
        assertFalse(local.incomingDir.exists() && local.incomingDir.list()!!.isNotEmpty())
        val edited = big.copyOf(2_000_001)
        local.writeLocalFile("gros.bin", edited)
        engine.sync(cfg)
        assertEquals(fileHash(edited), pc.one("SELECT hash FROM files WHERE path = 'gros.bin'"))
        assertEquals(2_000_001L, pc.one("SELECT length(data)::bigint FROM files WHERE path = 'gros.bin'"))
    }

    /** Fichier modifié des deux côtés, deux fois : chaque version de l'appareil a sa copie. */
    @Test
    fun fileConflictCopiesAreNotOverwritten() {
        val v0 = "{\"version\":1,\"elements\":[]}".toByteArray()
        pc.exec("INSERT INTO files (path, data, hash, size) VALUES ('d.mnemo-board', ?, ?, ?)", v0, fileHash(v0), v0.size.toLong())
        engine.sync(cfg)
        for (i in 1..2) {
            val mine = "{\"version\":1,\"elements\":[], \"tel\": $i}".toByteArray()
            val theirs = "{\"version\":1,\"elements\":[], \"pc\": $i}".toByteArray()
            local.writeLocalFile("d.mnemo-board", mine)
            pc.exec("UPDATE files SET data = ?, hash = ?, size = ? WHERE path = 'd.mnemo-board'", theirs, fileHash(theirs), theirs.size.toLong())
            engine.sync(cfg)
            assertEquals(String(theirs), String(local.fileOnDisk("d.mnemo-board").readBytes()))
            assertEquals(0, local.pendingCount())
        }
        assertTrue(String(local.fileOnDisk("d (conflit mobile).mnemo-board").readBytes()).contains("\"tel\": 1"))
        assertTrue(String(local.fileOnDisk("d (conflit mobile) 2.mnemo-board").readBytes()).contains("\"tel\": 2"))
        assertEquals(3, count("SELECT count(*) FROM files"))
    }

    /** Changer de base : rien de non envoyé n'est effacé, et cela part dans la nouvelle base. */
    @Test
    fun switchingDatabaseKeepsUnsentWork() {
        val plan = local.noteByPath("Plan")!!
        local.saveEdit(plan.id, plan.path, "v1", "pas encore envoyé")
        local.writeLocalFile("d.mnemo-board", Sketch().toJson().toByteArray())
        val here = engine.identity(cfg)
        assertNotNull(here.id)
        assertTrue(here.sameAs(engine.identity(parseDbUrl("$server/$dbName?sslmode=disable")!!)))

        // une autre base, vide de notes
        admin("postgres").use { c ->
            c.exec("DROP DATABASE IF EXISTS ${dbName}_2")
            c.exec("CREATE DATABASE ${dbName}_2")
        }
        val cfg2 = parseDbUrl("$server/${dbName}_2")!!
        admin("${dbName}_2").use { pc2 ->
            pc2.createStatement().use { it.execute(File("../../src/schema.sql").readText()) }
            assertFalse(here.sameAs(engine.identity(cfg2)))
            assertEquals(2, local.forgetRemote())
            assertEquals(listOf("Plan"), local.notes().map { it.path }) // « Accueil », simple copie, est oubliée
            engine.sync(cfg2)
            assertEquals("pas encore envoyé", pc2.one("SELECT content FROM notes WHERE path = 'Plan'"))
            assertEquals(1, (pc2.one("SELECT count(*) FROM files") as Number).toInt())
            assertEquals(0, local.pendingCount())
        }
        assertEquals("v1", pc.one("SELECT content FROM notes WHERE path = 'Plan'")) // l'ancienne base n'a pas bougé
    }
}
