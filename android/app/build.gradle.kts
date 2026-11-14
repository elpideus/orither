plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.kotlin.serialization)
}

/** 1.2.3 is 1002003, so every release installs over the one before. A pre-release shares its release's code. */
fun versionCodeOf(version: String): Int {
    val (major, minor, patch) = version.substringBefore('-').split('.').map(String::toInt)
    return major * 1_000_000 + minor * 1_000 + patch
}

// Releases are signed with Orither's own key, which the release workflow passes in
// (docs/releasing.md). Without it, every build is signed with this computer's debug key.
val keystore = providers.environmentVariable("ORITHER_KEYSTORE").orNull?.let(::file)?.takeIf { it.isFile }

android {
    namespace = "dev.orither"
    compileSdk { version = release(37) { minorApiLevel = 2 } }
    ndkVersion = "30.0.16248370"

    defaultConfig {
        applicationId = "dev.orither"
        minSdk = 29
        targetSdk = 36
        versionName = "0.2.0"
        versionCode = versionCodeOf(versionName!!)
        ndk { abiFilters += "arm64-v8a" }
    }

    signingConfigs {
        if (keystore != null) {
            create("release") {
                storeFile = keystore
                storePassword = providers.environmentVariable("ORITHER_KEYSTORE_PASSWORD").get()
                keyAlias = providers.environmentVariable("ORITHER_KEY_ALIAS").get()
                keyPassword = providers.environmentVariable("ORITHER_KEY_PASSWORD").get()
            }
        }
    }

    buildTypes {
        release {
            // Off until a shrunk build has been tried on a phone: the engine and the link are
            // reached by name, from app_process and from Rust, and proguard-rules.pro keeps them.
            isMinifyEnabled = false
            isShrinkResources = false
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            signingConfig = signingConfigs.findByName("release") ?: signingConfigs.getByName("debug")
        }
    }

    buildFeatures { compose = true }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_21
        targetCompatibility = JavaVersion.VERSION_21
    }

    sourceSets["main"].jniLibs.directories += layout.buildDirectory.dir("rust").get().asFile.path
    sourceSets["main"].assets.directories += layout.buildDirectory.dir("legal").get().asFile.path
}

// The Rust side of the link is built as part of the app.
val buildRust = tasks.register<Exec>("buildRust") {
    val output = layout.buildDirectory.dir("rust").get().asFile.path
    workingDir = rootDir.parentFile
    environment("ANDROID_NDK_HOME", androidComponents.sdkComponents.ndkDirectory.get().asFile.path)
    commandLine("cargo", "ndk", "-t", "arm64-v8a", "-o", output, "build", "--release", "-p", "orither-ffi")
}

// The licences travel with the app: Orither's, and the third-party ones NOTICE points to.
val copyLegal = tasks.register<Copy>("copyLegal") {
    from(rootDir.resolve("../LICENSE"), rootDir.resolve("../NOTICE"))
    from(rootDir.resolve("../licenses")) { into("licenses") }
    into(layout.buildDirectory.dir("legal"))
}

tasks.named("preBuild") { dependsOn(buildRust, copyLegal) }

dependencies {
    implementation(platform(libs.compose.bom))
    implementation(libs.compose.foundation)
    implementation(libs.compose.ui)
    implementation(libs.compose.material3)
    implementation(libs.activity.compose)
    implementation(libs.core.ktx)
    implementation(libs.lifecycle.compose)
    implementation(libs.camera.camera2)
    implementation(libs.camera.lifecycle)
    implementation(libs.camera.view)
    implementation(libs.coroutines.android)
    implementation(libs.serialization.json)
    implementation(libs.zxing.core)
    testImplementation(libs.kotlin.test)
}
