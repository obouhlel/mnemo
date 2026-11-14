package dev.mnemo.app

import android.graphics.Bitmap
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

    private fun waitText(text: String, timeout: Long = 60_000) =
        compose.waitUntil(timeout) { compose.onAllNodesWithText(text, substring = true).fetchSemanticsNodes().isNotEmpty() }

    @Test
    fun connectSyncAndRead() {
        val url = InstrumentationRegistry.getArguments().getString("dbUrl")
            ?: "postgres://postgres:postgres@10.0.2.2:5432/mnemo_ci"
        waitText("Base de synchronisation")
        shot("1-premier-lancement")
        compose.onNode(hasSetTextAction()).performTextInput(url)
        compose.onNodeWithText("Connecter").performClick()
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
