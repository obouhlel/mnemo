package dev.mnemo.app

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.viewModels
import dev.mnemo.app.ui.AppModel
import dev.mnemo.app.ui.MnemoApp
import dev.mnemo.app.ui.MnemoTheme

class MainActivity : ComponentActivity() {
    private val model: AppModel by viewModels()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        android.util.Log.i("Mnemo", "activité créée" + if (savedInstanceState != null) " (recréée)" else "")
        setContent { MnemoTheme { MnemoApp(model) } }
    }

    // synchronisation régulière seulement quand l'appli est visible (batterie, données)
    override fun onStart() {
        super.onStart()
        model.startLoop()
    }

    override fun onStop() {
        super.onStop()
        model.stopLoop()
    }
}
