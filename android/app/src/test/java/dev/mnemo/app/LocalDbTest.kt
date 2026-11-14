package dev.mnemo.app

import androidx.test.core.app.ApplicationProvider
import dev.mnemo.app.data.FileRow
import dev.mnemo.app.data.LocalDb
import dev.mnemo.app.data.fileHash
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import java.io.File
import java.util.UUID

/** Copie locale : l'écran et la synchronisation écrivent en même temps sans s'écraser. */
@RunWith(RobolectricTestRunner::class)
class LocalDbTest {
    private lateinit var db: LocalDb
    private val id = UUID.randomUUID().toString()

    @Before
    fun setUp() {
        db = LocalDb(ApplicationProvider.getApplicationContext(), "local-${UUID.randomUUID()}.db")
        db.vaultDir.deleteRecursively()
        db.vaultDir.mkdirs()
    }

    @Test
    fun settleOnlyClearsWhatWasPushed() {
        db.saveLocal(id, "Note", "envoyé")
        val pushed = db.note(id)!!
        db.saveEdit(id, "Note", "envoyé", "tapé pendant l'envoi")
        db.settle(pushed, 4)
        // la version suit la base, mais la note reste à envoyer avec son nouveau texte
        assertEquals(4, db.note(id)!!.version)
        assertTrue(db.note(id)!!.dirty)
        assertEquals("tapé pendant l'envoi", db.note(id)!!.content)
        db.settle(db.note(id)!!, 5)
        assertFalse(db.note(id)!!.dirty)
        assertEquals(5, db.note(id)!!.version)
    }

    @Test
    fun settleAdoptsThePathKeptByTheServer() {
        db.saveLocal(id, "Pris", "x")
        db.settle(db.note(id)!!, 1, "Pris (mobile)")
        assertEquals("Pris (mobile)", db.note(id)!!.path)
        assertFalse(db.note(id)!!.dirty)
        // renommée ici entre-temps : le nom choisi sur l'appareil n'est pas remplacé
        db.rename(id, "Autre")
        db.settle(db.note(id)!!.copy(path = "Pris (mobile)"), 2, "Pris (mobile 2)")
        assertEquals("Autre", db.note(id)!!.path)
        assertTrue(db.note(id)!!.dirty)
    }

    @Test
    fun remoteNeverReplacesUnsentWork() {
        assertTrue(db.putRemote(id, "Note", "v1", 1, 0))
        assertTrue(db.putRemote(id, "Note", "v2", 2, 0))
        db.saveEdit(id, "Note", "v2", "sur le téléphone")
        assertFalse(db.putRemote(id, "Note", "v3", 3, 0))
        assertEquals("sur le téléphone", db.note(id)!!.content)
        assertFalse(db.removeIfClean(id))
        // un conflit n'est réglé que si la note est restée telle qu'envoyée
        val pushed = db.note(id)!!
        db.saveEdit(id, "Note", "sur le téléphone", "encore modifiée")
        assertFalse(db.resolveConflict(pushed))
        assertTrue(db.resolveConflict(db.note(id)!!))
        assertTrue(db.putRemote(id, "Note", "v3", 3, 0))
        db.markDeleted(id)
        db.removeDeleted(id)
        assertNull(db.note(id))
    }

    @Test
    fun staleEditorSaveBecomesAConflictCopy() {
        db.putRemote(id, "Dossier/Note", "v1", 1, 0)
        // l'éditeur a chargé « v1 » ; la note change (reçue du bureau) avant qu'il enregistre
        db.putRemote(id, "Dossier/Note", "bureau", 2, 0)
        val first = db.saveEdit(id, "Dossier/Note", "v1", "v1 + téléphone")
        val second = db.saveEdit(id, "Dossier/Note", "v1", "v1 + téléphone, suite")
        assertTrue(first != id && second != id && first != second)
        assertEquals("bureau", db.note(id)!!.content)
        assertFalse(db.note(id)!!.dirty)
        val copies = listOf(db.note(first)!!, db.note(second)!!)
        assertTrue(copies.all { it.path.startsWith("Dossier/Note (conflit mobile ") && it.dirty && it.version == 0 })
        assertEquals(2, copies.map { it.path.lowercase() }.toSet().size) // même seconde : noms distincts
        // même texte que la base, ou base inchangée : pas de copie
        assertEquals(id, db.saveEdit(id, "Dossier/Note", "v1", "bureau"))
        assertEquals(id, db.saveEdit(id, "Dossier/Note", "bureau", "bureau + suite"))
        assertEquals(2, db.note(id)!!.version)
    }

    @Test
    fun editorSaveAfterDeletion() {
        db.putRemote(id, "Note", "v1", 1, 0)
        db.markDeleted(id)
        db.saveEdit(id, "Note", "v1", "saisie en attente") // supprimée ici : pas ressuscitée
        assertTrue(db.note(id)!!.deleted)
        db.removeDeleted(id)
        db.saveEdit(id, "Note", "v1", "saisie en attente") // supprimée ailleurs pendant la saisie : gardée
        assertEquals("saisie en attente", db.note(id)!!.content)
        assertEquals(0, db.note(id)!!.version)
    }

    @Test
    fun pathsCompareWithoutCaseBeyondAscii() {
        db.putRemote(id, "École/Été", "x", 1, 0)
        db.putRemote(UUID.randomUUID().toString(), "zèbre", "x", 1, 0)
        db.putRemote(UUID.randomUUID().toString(), "Avion", "x", 1, 0)
        assertEquals(id, db.noteByPath("école/été")!!.id)
        assertEquals(id, db.noteByPath("ÉCOLE/ÉTÉ")!!.id)
        assertEquals(listOf("Avion", "zèbre", "École/Été"), db.noteList().map { it.path })
        assertEquals(listOf(id), db.search("éTé").map { it.id })
        assertEquals(listOf("Avion"), db.search("avi").map { it.path })
    }

    @Test
    fun filesStayInsideTheVault() {
        for (p in listOf("../x.png", "a/../../x.png", "/etc/x", "a//b", "c:x", "a\\b", ".caché/x", "a/.x", "")) {
            assertNull(p, db.localFile(p))
            assertTrue(p, runCatching { db.fileOnDisk(p) }.isFailure)
            assertTrue(p, runCatching { db.writeLocalFile(p, byteArrayOf(1)) }.isFailure)
        }
        db.writeLocalFile("Pièces jointes/a b.png", byteArrayOf(1))
        assertNotNull(db.localFile("Pièces jointes/a b.png"))
        assertFalse(File(db.vaultDir.parentFile, "x.png").exists())
    }

    @Test
    fun fileSavedDuringUploadStaysDirty() {
        val v1 = "un".toByteArray()
        val v2 = "deux".toByteArray()
        db.writeLocalFile("d.mnemo-board", v1)
        db.writeLocalFile("d.mnemo-board", v2) // enregistré pendant l'envoi de v1
        db.settleFile("d.mnemo-board", fileHash(v1), fileHash(v1))
        var row = db.fileRow("d.mnemo-board")!!
        assertTrue(row.dirty)
        assertEquals(fileHash(v1), row.hash)
        assertEquals(fileHash(v2), row.localHash)
        // une version reçue de la base ne remplace pas un fichier à envoyer
        val tmp = File(db.vaultDir.parentFile, "tmp-test").apply { writeText("base") }
        assertFalse(db.installRemote("d.mnemo-board", "h", 4, tmp))
        assertFalse(tmp.exists())
        assertEquals("deux", db.fileOnDisk("d.mnemo-board").readText())
        assertFalse(db.removeFileIfClean("d.mnemo-board"))
        db.settleFile("d.mnemo-board", fileHash(v2), fileHash(v2))
        row = db.fileRow("d.mnemo-board")!!
        assertFalse(row.dirty)
        // à jour : la version de la base s'installe, d'un seul coup
        val tmp2 = File(db.vaultDir.parentFile, "tmp-test").apply { writeText("base") }
        assertTrue(db.installRemote("d.mnemo-board", "h", 4, tmp2))
        assertEquals("base", db.fileOnDisk("d.mnemo-board").readText())
        assertEquals(FileRow("d.mnemo-board", "h", 4, "h", dirty = false, deleted = false), db.fileRow("d.mnemo-board"))
    }

    @Test
    fun staleBoardSaveGoesToACopy() {
        val loaded = "chargé".toByteArray()
        db.writeLocalFile("d.mnemo-board", loaded)
        db.settleFile("d.mnemo-board", fileHash(loaded), fileHash(loaded))
        // le fichier change (reçu du bureau) alors que l'écran a encore l'ancien dessin
        val tmp = File(db.vaultDir.parentFile, "tmp-test").apply { writeText("bureau") }
        db.installRemote("d.mnemo-board", fileHash("bureau".toByteArray()), 6, tmp)
        val target = db.writeLocalFile("d.mnemo-board", "chargé + trait".toByteArray(), fileHash(loaded))
        assertEquals("d (conflit mobile).mnemo-board", target)
        assertEquals("bureau", db.fileOnDisk("d.mnemo-board").readText())
        assertFalse(db.fileRow("d.mnemo-board")!!.dirty)
        assertEquals("chargé + trait", db.fileOnDisk(target).readText())
        assertTrue(db.fileRow(target)!!.dirty)
        // enregistrement normal : même fichier
        assertEquals(target, db.writeLocalFile(target, "suite".toByteArray(), fileHash("chargé + trait".toByteArray())))
    }

    @Test
    fun forgetRemoteKeepsUnsentWork() {
        db.putRemote(id, "Copie", "de la base", 3, 0)
        val edited = UUID.randomUUID().toString()
        db.putRemote(edited, "Modifiée", "v1", 7, 0)
        db.saveEdit(edited, "Modifiée", "v1", "pas envoyée")
        val fresh = UUID.randomUUID().toString()
        db.saveLocal(fresh, "Neuve", "jamais envoyée")
        db.writeLocalFile("neuf.mnemo-board", byteArrayOf(1))
        val tmp = File(db.vaultDir.parentFile, "tmp-test").apply { writeText("base") }
        db.installRemote("copie.png", "h", 4, tmp)
        db.replaceFolders(listOf("Vide"))

        assertEquals(3, db.forgetRemote())
        assertEquals(listOf("Modifiée", "Neuve"), db.noteList().map { it.path })
        assertTrue(db.notes().all { it.dirty && it.version == 0 })
        assertEquals("pas envoyée", db.note(edited)!!.content)
        assertEquals(listOf(FileRow("neuf.mnemo-board", null, 1, fileHash(byteArrayOf(1)), dirty = true, deleted = false)), db.files())
        assertNull(db.localFile("copie.png"))
        assertTrue(db.folders().isEmpty())
    }
}
