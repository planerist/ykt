import org.gradle.jvm.toolchain.JvmVendorSpec.ADOPTIUM

plugins {
    val kotlinVersion = tools.versions.kotlin

    base
    kotlin("jvm") version kotlinVersion apply true
}

group = "com.planerist.ykt"

val compilationJdkTarget: String = tools.versions.jdk.get()
val jdkVendor = ADOPTIUM

kotlin {
    jvmToolchain {
        languageVersion.set(JavaLanguageVersion.of(compilationJdkTarget))
        vendor.set(jdkVendor)
    }
}

java {
    toolchain {
        languageVersion.set(JavaLanguageVersion.of(compilationJdkTarget))
        vendor.set(jdkVendor)
    }
}

allprojects {
    group = rootProject.group
    version = rootProject.version

    repositories {
        mavenCentral()
        google()
        mavenLocal()
    }
}