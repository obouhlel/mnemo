package dev.mnemo.app

import android.app.Application
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Paint
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onRoot
import androidx.test.core.app.ApplicationProvider
import com.github.takahirom.roborazzi.captureRoboImage
import dev.mnemo.app.data.Element
import dev.mnemo.app.data.Pt
import dev.mnemo.app.data.Sketch
import dev.mnemo.app.ui.AppModel
import dev.mnemo.app.ui.MnemoApp
import dev.mnemo.app.ui.MnemoTheme
import dev.mnemo.app.ui.Screen
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import java.io.ByteArrayOutputStream
import java.util.UUID

/** Captures des écrans (téléphone et tablette) avec des notes d'exemple, dans app/build/screens/. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class ScreensTest {
    @get:Rule
    val compose = createComposeRule()

    private fun seeded(): AppModel {
        val app = ApplicationProvider.getApplicationContext<Application>()
        val model = AppModel(app, "screens-${UUID.randomUUID()}.db")
        val db = model.local
        app.getSharedPreferences("mnemo", 0).edit().putString("db_url", "postgres://moi@db.exemple.fr/vault").apply()
        fun note(path: String, content: String) = db.putRemote(UUID.randomUUID().toString(), path, content, 1, 0)
        note("Accueil", "Bienvenue dans **Mnemo** sur mobile.\n\n- [x] Synchronisation\n- [ ] Écrire depuis la tablette\n\nVoir [[Plan]] et #projet")
        note("Projets/Plan", "# Plan\n\n![[schema.png]]\n\nÉtapes suivantes : *Android*.")
        note("Projets/Mnemo/Architecture", "Notes du projet")
        note("Journal/2026-11-14", "Réunion")
        // image : un schéma simple
        val bmp = Bitmap.createBitmap(420, 200, Bitmap.Config.ARGB_8888)
        Canvas(bmp).apply {
            drawColor(0xFF2A2C3C.toInt())
            val p = Paint().apply { color = 0xFF8B6CEF.toInt(); style = Paint.Style.STROKE; strokeWidth = 8f }
            drawRect(20f, 20f, 400f, 180f, p)
            drawCircle(210f, 100f, 50f, Paint().apply { color = 0xFF44CF6E.toInt() })
        }
        val png = ByteArrayOutputStream().also { bmp.compress(Bitmap.CompressFormat.PNG, 100, it) }.toByteArray()
        db.writeLocalFile("Pièces jointes/schema.png", png)
        val board = Sketch(
            listOf(
                Element.Shape("#1e1e1e", 2f, 0f, 0f, 160f, 90f, false),
                Element.Line("#1971c2", 2f, listOf(Pt(170f, 45f), Pt(260f, 45f)), true),
                Element.Shape("#e03131", 2f, 270f, 0f, 120f, 90f, true),
                Element.Text("#2f9e44", 20f, 10f, 110f, "Serveur"),
                Element.Stroke("#f08c00", 4f, (0..30).map { Pt(10f + it * 12f, 160f + if (it % 2 == 0) 0f else 18f) }),
            ),
        )
        db.writeLocalFile("Projets/Schéma.mnemo-board", board.toJson().toByteArray())
        db.setMeta(
            "board",
            JSONObject().put("projects", JSONArray().put(JSONObject().put("id", "p1").put("name", "Mnemo").put("repo", "obouhlel/mnemo")))
                .put(
                    "tickets",
                    JSONArray()
                        .put(ticket("Appli Android", "doing", listOf("enhancement"), 17, false))
                        .put(ticket("Images dans les notes", "done", listOf("enhancement"), 15, true))
                        .put(ticket("Whiteboard", "done", emptyList(), 16, true))
                        .put(ticket("Thème clair sur mobile", "todo", listOf("good first issue"), null, false)),
                ).toString(),
        )
        model.reload()
        return model
    }

    private fun ticket(title: String, status: String, tags: List<String>, gh: Int?, pr: Boolean) =
        JSONObject().put("id", UUID.randomUUID().toString()).put("project", "p1").put("number", 1).put("title", title)
            .put("body", "").put("status", status).put("tags", JSONArray(tags)).put("priority", 1)
            .put("github", gh ?: JSONObject.NULL).put("state", if (status == "done") "merged" else "open").put("pr", pr)

    private fun shot(model: AppModel, name: String, screen: Screen? = null, dark: Boolean = true) {
        screen?.let { model.open(it) }
        compose.setContent { MnemoTheme(dark = dark) { MnemoApp(model) } }
        compose.waitForIdle()
        // images de Markwon chargées en arrière-plan, puis affichées sur le fil principal
        repeat(5) {
            Thread.sleep(150)
            org.robolectric.Shadows.shadowOf(android.os.Looper.getMainLooper()).idle()
            compose.waitForIdle()
        }
        compose.onRoot().captureRoboImage("build/screens/$name.png")
    }

    @Test
    @Config(qualifiers = "w411dp-h891dp-xxhdpi")
    fun phoneList() = shot(seeded(), "phone-list")

    @Test
    @Config(qualifiers = "w411dp-h891dp-xxhdpi")
    fun phoneNote() {
        val m = seeded()
        shot(m, "phone-note", Screen.Note(m.local.noteByPath("Projets/Plan")!!.id))
    }

    @Test
    @Config(qualifiers = "w411dp-h891dp-xxhdpi")
    fun phoneBoard() = shot(seeded(), "phone-board", Screen.Board("Projets/Schéma.mnemo-board"))

    @Test
    @Config(qualifiers = "w1280dp-h800dp-mdpi")
    fun tabletNote() {
        val m = seeded()
        shot(m, "tablet-note", Screen.Note(m.local.noteByPath("Accueil")!!.id), dark = false)
    }

    @Test
    @Config(qualifiers = "w1280dp-h800dp-mdpi")
    fun tabletDashboard() = shot(seeded(), "tablet-dashboard", Screen.Dashboard)

    /** L'activité démarre comme sur un téléphone (ViewModel créé par Android, par réflexion). */
    @Test
    fun activityStarts() {
        androidx.test.core.app.ActivityScenario.launch(MainActivity::class.java).use { sc ->
            sc.moveToState(androidx.lifecycle.Lifecycle.State.RESUMED)
            sc.onActivity { a -> org.junit.Assert.assertFalse(a.isFinishing) }
        }
    }

    @Test
    @Config(qualifiers = "w411dp-h891dp-xxhdpi")
    fun phoneFirstRun() {
        val app = ApplicationProvider.getApplicationContext<Application>()
        app.getSharedPreferences("mnemo", 0).edit().clear().apply()
        shot(AppModel(app, "empty-${UUID.randomUUID()}.db"), "phone-first-run")
    }
}
