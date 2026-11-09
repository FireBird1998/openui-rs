plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "dev.openui.demo"
    compileSdk = 36

    defaultConfig {
        applicationId = "dev.openui.demo"
        // The generated bindings free Rust objects with Android's own
        // SystemCleaner, which needs Android 14.
        minSdk = 34
        targetSdk = 36
        versionCode = 1
        versionName = "0.1"
        ndk { abiFilters += "arm64-v8a" }
    }

    buildFeatures { compose = true }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

kotlin {
    compilerOptions { jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17) }
}

dependencies {
    implementation("androidx.activity:activity-compose:1.10.1")
    implementation("androidx.compose.foundation:foundation:1.9.0")
    implementation("androidx.compose.ui:ui:1.9.0")
    implementation("androidx.annotation:annotation:1.9.1")
    // UniFFI's Kotlin bindings call the Rust library through JNA; its .aar
    // carries the Android builds of its native library. JNA is LGPL-2.1 or
    // Apache-2.0; this uses it under Apache-2.0.
    implementation("net.java.dev.jna:jna:5.17.0@aar")
}
