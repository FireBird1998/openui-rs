plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.plugin.compose")
}

// scripts/kotlin-check.sh generates the binding and builds the core, and
// passes where it put them. Run on their own, these defaults are where the
// script puts them with the default Cargo target folder.
val repo = rootDir.parentFile
val bindingDir = file(providers.gradleProperty("openui.bindingDir").getOrElse("$repo/target/kotlin-binding"))
val hostLibDir = file(providers.gradleProperty("openui.hostLibDir").getOrElse("$repo/target/debug"))

android {
    namespace = "dev.openui.compose"
    compileSdk = 36

    defaultConfig {
        // The floor the spec sets (#257): Android 14.
        minSdk = 34
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    buildFeatures { compose = true }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    sourceSets {
        named("main") {
            // The generated binding, every declaration internal
            // (scripts/kotlin-binding.py), and the core built for Android.
            kotlin.directories.add("$bindingDir/kotlin")
            jniLibs.directories.add("$bindingDir/jniLibs")
        }
    }

    testOptions {
        unitTests.all { test ->
            // The JVM tests load the core built for this machine (a .dylib
            // on a Mac, a .so on Linux) through JNA, from this folder.
            val hostLib = fileTree(hostLibDir) { include("libopenui_ffi.dylib", "libopenui_ffi.so") }
            test.inputs.files(hostLib).withPropertyName("hostLib")
            test.systemProperty("jna.library.path", hostLibDir.path)
            test.testLogging { events("passed", "skipped", "failed") }
            test.doFirst {
                check(!hostLib.isEmpty) {
                    "no libopenui_ffi.dylib or libopenui_ffi.so in $hostLibDir: run scripts/kotlin-check.sh, " +
                        "or pass -Popenui.hostLibDir"
                }
            }
        }
    }
}

kotlin {
    // Every public declaration says so: the compiler refuses one that
    // doesn't, so nothing becomes public API by accident (the generated
    // binding is all internal; scripts/kotlin-binding.py).
    explicitApi()
    compilerOptions { jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17) }
}

// Every library here is checked against the approved licences
// (licences.gradle.kts): what ships, and what only the tests use.
dependencies {
    implementation("androidx.compose.material3:material3:1.4.0")
    // Compose itself at the version material3 1.4.0 was released with,
    // so its parts don't mix 1.8 and 1.9.
    implementation("androidx.compose.foundation:foundation:1.9.0")
    implementation("androidx.compose.ui:ui:1.9.0")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.10.2")
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.9.0")
    // UniFFI's Kotlin bindings call the core through JNA; its .aar carries
    // the Android builds of JNA's own native library. JNA is LGPL-2.1 or
    // Apache-2.0; this uses it under Apache-2.0.
    implementation("net.java.dev.jna:jna:5.17.0@aar")

    // The JVM tests: JNA's .jar carries its native library for desktop
    // systems, which the .aar doesn't.
    testImplementation("junit:junit:4.13.2")
    testImplementation("net.java.dev.jna:jna:5.17.0")

    androidTestImplementation("androidx.test:runner:1.6.2")
    androidTestImplementation("androidx.test.ext:junit:1.2.1")
    androidTestImplementation("androidx.compose.ui:ui-test-junit4:1.9.0")
    // The empty activity the Compose test rule starts; debug builds only.
    debugImplementation("androidx.compose.ui:ui-test-manifest:1.9.0")
}

// The licence check and its self-test: licences.gradle.kts.
apply(from = "licences.gradle.kts")

