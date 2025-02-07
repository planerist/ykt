plugins {
    kotlin("jvm") apply true
}

version = "1.0-SNAPSHOT"

dependencies {
    implementation("net.java.dev.jna:jna:5.16.0")
    testImplementation(kotlin("test"))
}

tasks.test {
    useJUnitPlatform()
}

kotlin {
    jvmToolchain(21)
}

val os = org.gradle.internal.os.OperatingSystem.current()
val ext = if(os.isLinux) "so" else "dylib"
val dir = if(os.isLinux)  "linux-x86-64" else "darwin-x86-64"

task<Exec>("buildRust") {
    executable = "sh"
    args("../build_bindings.sh", ext, dir)
}