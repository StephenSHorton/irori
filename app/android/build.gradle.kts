plugins {
    id("com.android.application") version "8.11.0"
    id("org.jetbrains.kotlin.android") version "2.0.21"
}

android {
    namespace = "com.stephenshorton.irori.rune"
    compileSdk = 35
    defaultConfig {
        // Its own id, so it never replaces an installed Tauri build
        // (com.stephenshorton.irori) or shares its data.
        applicationId = "com.stephenshorton.irori.rune"
        minSdk = 26
        targetSdk = 35
        versionCode = 1
        versionName = "0.0.1-rune"
        ndk { abiFilters += "arm64-v8a" }
    }
    sourceSets["main"].apply {
        // Rune's Kotlin host from cargo's checkout of the pinned Rune rev
        // (app/Cargo.toml), so its keyboard protocol matches the Rust side.
        java.srcDir(providers.gradleProperty("rune.kotlin").get())
        // libirori_app.so, staged by scripts/android-rune.sh.
        jniLibs.srcDir(providers.gradleProperty("rune.jniLibs").getOrElse("build/rune-jni"))
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

kotlin { jvmToolchain(17) }
