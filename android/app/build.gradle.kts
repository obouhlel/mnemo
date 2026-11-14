plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
    id("io.github.takahirom.roborazzi")
}

// Version de l'appli = version de Mnemo (Cargo.toml), pour que l'APK suive les releases
val mnemoVersion: String = rootProject.file("../Cargo.toml").readLines()
    .first { it.startsWith("version = ") }.substringAfter("\"").substringBefore("\"")

android {
    namespace = "dev.mnemo.app"
    compileSdk = 35

    defaultConfig {
        applicationId = "dev.mnemo.app"
        minSdk = 26 // Android 8.0
        targetSdk = 35
        val (maj, min, pat) = mnemoVersion.split(".").map { it.toInt() }
        versionCode = maj * 10000 + min * 100 + pat
        versionName = mnemoVersion
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    // Clé fournie par la CI (secrets) ; sinon la clé de débogage de la machine, pour une installation directe de l'APK
    val keystore = System.getenv("MNEMO_KEYSTORE")?.let { file(it) }?.takeIf { it.exists() }
    signingConfigs {
        if (keystore != null) {
            create("release") {
                storeFile = keystore
                storePassword = System.getenv("MNEMO_KEYSTORE_PASSWORD")
                keyAlias = System.getenv("MNEMO_KEY_ALIAS")
                keyPassword = System.getenv("MNEMO_KEY_PASSWORD")
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            signingConfig = signingConfigs.getByName(if (keystore != null) "release" else "debug")
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
        isCoreLibraryDesugaringEnabled = true
    }
    kotlinOptions {
        jvmTarget = "17"
    }
    buildFeatures {
        compose = true
    }
    testOptions {
        unitTests {
            isIncludeAndroidResources = true
            all { it.systemProperty("robolectric.graphicsMode", "NATIVE") }
        }
    }
    packaging {
        resources.excludes += setOf("META-INF/LICENSE*", "META-INF/NOTICE*", "META-INF/*.kotlin_module")
    }
}

// Le pilote PostgreSQL lit la taille du tas via java.lang.management, absent d'Android (plantage à la
// connexion). On le reconditionne sans PGPropertyMaxResultBufferParser, remplacée par la copie corrigée
// de src/main/java/org/postgresql/util.
val pgjdbc: Configuration by configurations.creating { isTransitive = false }
val pgjdbcAndroid = tasks.register<Jar>("pgjdbcAndroid") {
    archiveFileName.set("postgresql-android.jar")
    destinationDirectory.set(layout.buildDirectory.dir("pgjdbc"))
    from({ pgjdbc.map { zipTree(it) } }) {
        exclude("org/postgresql/util/PGPropertyMaxResultBufferParser*.class")
        exclude("META-INF/*.SF", "META-INF/*.RSA", "META-INF/*.DSA")
    }
    duplicatesStrategy = DuplicatesStrategy.EXCLUDE
}

dependencies {
    pgjdbc("org.postgresql:postgresql:42.2.29")

    coreLibraryDesugaring("com.android.tools:desugar_jdk_libs:2.1.3")

    val composeBom = platform("androidx.compose:compose-bom:2024.10.01")
    implementation(composeBom)
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.ui:ui-tooling-preview")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.material:material-icons-extended")
    implementation("androidx.activity:activity-compose:1.9.3")
    implementation("androidx.core:core-ktx:1.13.1")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.8.7")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.8.7")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.9.0")

    // PostgreSQL (même base que Mnemo sur ordinateur) : pilote retouché pour Android, voir pgjdbcAndroid
    implementation(files(pgjdbcAndroid))
    // affichage Markdown
    implementation("io.noties.markwon:core:4.6.2")
    implementation("io.noties.markwon:ext-strikethrough:4.6.2")
    implementation("io.noties.markwon:ext-tasklist:4.6.2")
    implementation("io.noties.markwon:ext-tables:4.6.2")

    testImplementation("junit:junit:4.13.2")
    testImplementation("org.json:json:20240303")
    testImplementation("org.robolectric:robolectric:4.14.1")
    testImplementation("androidx.test:core:1.6.1")
    testImplementation("androidx.compose.ui:ui-test-junit4")
    testImplementation("io.github.takahirom.roborazzi:roborazzi:1.43.0")
    testImplementation("io.github.takahirom.roborazzi:roborazzi-compose:1.43.0")
    testImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.9.0")
    androidTestImplementation(composeBom)
    androidTestImplementation("androidx.compose.ui:ui-test-junit4")
    androidTestImplementation("androidx.test.ext:junit:1.2.1")
    androidTestImplementation("androidx.test:runner:1.6.2")
    debugImplementation("androidx.compose.ui:ui-test-manifest")
    debugImplementation("androidx.compose.ui:ui-tooling")
}
