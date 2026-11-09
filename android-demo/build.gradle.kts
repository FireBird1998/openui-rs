// The Android Gradle plugin brings Kotlin with it (AGP 9's built-in Kotlin);
// the Compose compiler plugin must match that Kotlin version.
buildscript {
    repositories {
        google()
        mavenCentral()
    }
    dependencies {
        classpath("com.android.tools.build:gradle:9.2.1")
        classpath("org.jetbrains.kotlin:compose-compiler-gradle-plugin:2.2.10")
    }
}
