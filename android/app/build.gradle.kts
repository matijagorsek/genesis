plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "org.genesis.companion"
    compileSdk = 35
    defaultConfig {
        applicationId = "org.genesis.companion"
        minSdk = 26
        targetSdk = 35
        versionCode = 1
        versionName = "0.1"
    }
    // a release keystore outside the repository: ~/.config/genesis-android/keystore.properties
    val ksProps = java.util.Properties().apply {
        val f = java.io.File(System.getProperty("user.home"), ".config/genesis-android/keystore.properties")
        if (f.exists()) f.inputStream().use { load(it) }
    }
    signingConfigs {
        if (ksProps.containsKey("storeFile")) create("release") {
            storeFile = file(ksProps["storeFile"] as String); storePassword = ksProps["storePassword"] as String
            keyAlias = ksProps["keyAlias"] as String; keyPassword = ksProps["keyPassword"] as String
        }
    }
    buildTypes {
        release {
            isMinifyEnabled = false
            if (ksProps.containsKey("storeFile")) signingConfig = signingConfigs.getByName("release")
        }
    }
    compileOptions { sourceCompatibility = JavaVersion.VERSION_17; targetCompatibility = JavaVersion.VERSION_17 }
    kotlinOptions { jvmTarget = "17" }
    buildFeatures { compose = true }
}

dependencies {
    val composeBom = platform("androidx.compose:compose-bom:2024.10.01")
    implementation(composeBom)
    implementation("androidx.activity:activity-compose:1.9.3")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.ui:ui-tooling-preview")
    implementation("androidx.lifecycle:lifecycle-runtime-ktx:2.8.7")
    implementation("com.squareup.okhttp3:okhttp:4.12.0")
    implementation("com.journeyapps:zxing-android-embedded:4.3.0")
    implementation("org.json:json:20240303")
}
