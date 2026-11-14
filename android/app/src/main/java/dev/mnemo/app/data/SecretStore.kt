package dev.mnemo.app.data

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * Adresse de la base (elle contient le mot de passe) : chiffrée par une clé du Keystore Android, qui ne quitte pas
 * l'appareil, avant d'être rangée dans les préférences. Les préférences sont par ailleurs exclues des sauvegardes
 * (voir le manifeste). Si le Keystore est indisponible, l'adresse reste dans les préférences privées de l'appli.
 */
class SecretStore(context: Context) {
    private val prefs = context.getSharedPreferences("mnemo", Context.MODE_PRIVATE)

    private fun key(): SecretKey {
        val ks = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (ks.getKey(ALIAS, null) as? SecretKey)?.let { return it }
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore").run {
            init(
                KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                    .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                    .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                    .build(),
            )
            generateKey()
        }
    }

    fun dbUrl(): String {
        prefs.getString(ENCRYPTED, null)?.let { stored ->
            return runCatching {
                val raw = Base64.decode(stored, Base64.NO_WRAP)
                val cipher = Cipher.getInstance("AES/GCM/NoPadding")
                cipher.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, raw, 0, IV))
                String(cipher.doFinal(raw, IV, raw.size - IV), Charsets.UTF_8)
            }.onFailure { android.util.Log.w("Mnemo", "adresse de la base illisible (clé perdue) : à saisir de nouveau", it) }
                .getOrDefault("")
        }
        // adresse en clair d'une version précédente : chiffrée au passage
        val plain = prefs.getString(PLAIN, "") ?: ""
        if (plain.isNotEmpty()) setDbUrl(plain)
        return plain
    }

    fun setDbUrl(url: String?) {
        val edit = prefs.edit().remove(PLAIN).remove(ENCRYPTED)
        if (!url.isNullOrEmpty()) {
            runCatching {
                val cipher = Cipher.getInstance("AES/GCM/NoPadding")
                cipher.init(Cipher.ENCRYPT_MODE, key())
                Base64.encodeToString(cipher.iv + cipher.doFinal(url.toByteArray(Charsets.UTF_8)), Base64.NO_WRAP)
            }.onSuccess { edit.putString(ENCRYPTED, it) }
                .onFailure { edit.putString(PLAIN, url) } // pas de Keystore (tests hors appareil)
        }
        edit.apply()
    }

    private companion object {
        const val ALIAS = "mnemo.db_url"
        const val PLAIN = "db_url"
        const val ENCRYPTED = "db_url_enc"
        const val IV = 12
    }
}
