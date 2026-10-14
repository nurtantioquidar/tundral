import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    kotlin("jvm") version "2.0.0"
}

fun shopVersion(): String = "1.0"

version = shopVersion()

kotlin {
    compilerOptions { jvmTarget.set(JvmTarget.JVM_17) }
}
