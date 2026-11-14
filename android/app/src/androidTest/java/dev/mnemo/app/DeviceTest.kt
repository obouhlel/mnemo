package dev.mnemo.app

import android.graphics.Bitmap
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File

/**
 * Sur un vrai Android (émulateur API 26 en CI) : premier lancement, connexion à la base PostgreSQL de la CI
 * (10.0.2.2 = la machine hôte), synchronisation, ouverture d'une note avec son image. Captures dans
 * /sdcard/Android/data/dev.mnemo.app/files/ (récupérées par la CI).
 */
@RunWith(AndroidJUnit4::class)
class DeviceTest {
    @get:Rule
    val compose = createAndroidComposeRule<MainActivity>()

    private fun shot(name: String) {
        val inst = InstrumentationRegistry.getInstrumentation()
        inst.waitForIdleSync()
        val bmp = inst.uiAutomation.takeScreenshot() ?: return
        val dir = inst.targetContext.getExternalFilesDir(null) ?: return
        File(dir, "$name.png").outputStream().use { bmp.compress(Bitmap.CompressFormat.PNG, 100, it) }
    }

    private fun waitText(text: String, timeout: Long = 60_000) {
        try {
            compose.waitUntil(timeout) { compose.onAllNodesWithText(text, substring = true).fetchSemanticsNodes().isNotEmpty() }
        } catch (e: Throwable) {
            shot("echec-$text")
            val seen = compose.onAllNodes(androidx.compose.ui.test.isRoot()).fetchSemanticsNodes().flatMap { root ->
                fun walk(n: androidx.compose.ui.semantics.SemanticsNode): List<String> =
                    (n.config.getOrElseNullable(androidx.compose.ui.semantics.SemanticsProperties.Text) { null }?.map { it.text } ?: emptyList()) +
                        listOfNotNull(n.config.getOrElseNullable(androidx.compose.ui.semantics.SemanticsProperties.EditableText) { null }?.text?.let { "[champ: $it]" }) +
                        n.children.flatMap { walk(it) }
                walk(root)
            }
            throw AssertionError("« $text » introuvable ; à l'écran : $seen", e)
        }
    }

    private val url = InstrumentationRegistry.getArguments().getString("dbUrl")
        ?: "postgres://postgres:postgres@10.0.2.2:5432/mnemo_ci"

    /** Le pilote PostgreSQL sur le vrai runtime Android : connexion et synchronisation complètes. */
    @Test
    fun engineOnDevice() {
        val ctx = InstrumentationRegistry.getInstrumentation().targetContext
        val local = dev.mnemo.app.data.LocalDb(ctx, "device-engine.db")
        val engine = dev.mnemo.app.data.SyncEngine(local, "android-ci")
        val cfg = dev.mnemo.app.data.parseDbUrl(url)!!
        val err = engine.test(cfg)
        org.junit.Assert.assertNull("connexion : $err", err)
        try {
            engine.sync(cfg)
        } catch (e: Throwable) {
            throw AssertionError("synchronisation : " + e.stackTraceToString().take(3000), e)
        }
        org.junit.Assert.assertTrue(local.notes().any { it.path == "Projets/Plan" })
        org.junit.Assert.assertTrue(local.fileOnDisk("Pièces jointes/schema.png").exists())
    }

    @Test
    fun connectSyncAndRead() {
        waitText("Base de synchronisation")
        shot("1-premier-lancement")
        compose.onNodeWithText("Afficher l'adresse").performClick()
        compose.onNode(hasSetTextAction()).performTextInput(url)
        compose.waitForIdle()
        compose.onNode(hasSetTextAction()).assert(hasText(url))
        compose.onNodeWithText("Connecter").assertIsEnabled().performClick()
        waitText("Projets")
        shot("2-liste")
        compose.onNodeWithText("Projets").performClick()
        waitText("Plan")
        compose.onNode(hasText("Plan") and !hasSetTextAction()).performClick()
        waitText("Étapes suivantes")
        Thread.sleep(1500)
        shot("3-note-avec-image")
    }
}
