// The Android prototype around Irori's Rune window (card #680; not shipped,
// main's Tauri phone app stays): Rune's Kotlin host (RuneActivity, for the
// on-screen keyboard) and libirori_app.so, the cargo build of app/.
// scripts/android-rune.sh builds both.
pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}
dependencyResolutionManagement {
    repositories {
        google()
        mavenCentral()
    }
}
rootProject.name = "irori-rune"
