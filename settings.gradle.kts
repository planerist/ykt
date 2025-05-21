pluginManagement {
    repositories {
        mavenCentral()
        gradlePluginPortal()
        mavenLocal()
    }
}

plugins { id("org.gradle.toolchains.foojay-resolver") version "0.8.0" }

@Suppress("UnstableApiUsage")
toolchainManagement {
    jvm {
        javaRepositories {
            repository("foojay") {
                resolverClass.set(org.gradle.toolchains.foojay.FoojayToolchainResolver::class.java)
            }
        }
    }
}

val kotlinVersion = "2.1.21"

dependencyResolutionManagement {
    versionCatalogs {
        create("tools") {
            version("kotlin", kotlinVersion)
            version("jdk", "21")
        }
    }
}

rootProject.name = "ykt"

include("yrs_kt")
