//! How to build and open each project's example apps, as data. Another
//! project's layout is one more entry in [`RECIPES`]; the platform mechanics
//! (simulators, emulators, installing, launching) are generic and live in
//! `job.rs`.

use std::path::Path;

use crate::model::RunPlatform;

/// A script the recipe runs, relative to the source tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Script {
    /// Directory it runs in, relative to the source root.
    pub dir: &'static str,
    /// Program, relative to `dir` (`./build_ios.sh`).
    pub program: &'static str,
    /// Arguments. `{abi}` is replaced with the Android device's ABI as the
    /// script names it (see [`AndroidRecipe::abi_names`]).
    pub args: &'static [&'static str],
    /// Extra environment, e.g. to skip a step the job does itself.
    pub vars: &'static [(&'static str, &'static str)],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IosRecipe {
    /// Builds the native SDK for the simulator.
    pub sdk: Script,
    /// Directory `xcodebuild` runs in, relative to the source root.
    pub xcode_dir: &'static str,
    /// The `.xcodeproj`, relative to `xcode_dir`.
    pub project: &'static str,
    pub scheme: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AndroidRecipe {
    /// Builds the native library for one ABI.
    pub sdk: Script,
    /// Pairs of (Android ABI, the name the SDK script takes for it).
    pub abi_names: &'static [(&'static str, &'static str)],
    /// Directory holding `gradlew`, relative to the source root.
    pub gradle_dir: &'static str,
    /// Tasks that build the app and install it on `ANDROID_SERIAL`.
    pub gradle_tasks: &'static [&'static str],
    /// The app module's build file, where its `applicationId` is read.
    pub app_build_file: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecipeKind {
    Ios(IosRecipe),
    Android(AndroidRecipe),
}

/// A program the recipe needs on PATH.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tool {
    pub program: &'static str,
    /// Shown when it is missing.
    pub missing: &'static str,
}

const CARGO: Tool = Tool {
    program: "cargo",
    missing: "Rust (cargo) was not found.",
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Recipe {
    /// Short name for logs.
    pub name: &'static str,
    /// Files that must all exist in the source tree for the recipe to apply.
    pub markers: &'static [&'static str],
    /// Checked before a run is offered.
    pub requires: &'static [Tool],
    pub kind: RecipeKind,
}

impl Recipe {
    pub fn platform(&self) -> RunPlatform {
        match self.kind {
            RecipeKind::Ios(_) => RunPlatform::Ios,
            RecipeKind::Android(_) => RunPlatform::Android,
        }
    }

    /// Every marker exists under `root`.
    pub fn applies_to(&self, root: &Path) -> bool {
        self.markers.iter().all(|m| root.join(m).exists())
    }
}

/// Dash Platform's Swift SDK and SwiftExampleApp
/// (`packages/swift-sdk/BUILD_GUIDE_FOR_AI.md`).
const PLATFORM_IOS: Recipe = Recipe {
    name: "SwiftExampleApp",
    markers: &[
        "packages/swift-sdk/build_ios.sh",
        "packages/swift-sdk/SwiftExampleApp/SwiftExampleApp.xcodeproj",
    ],
    requires: &[CARGO],
    kind: RecipeKind::Ios(IosRecipe {
        // Release profile, the script's default: the dev profile's debug
        // assertions abort the host app.
        sdk: Script {
            dir: "packages/swift-sdk",
            program: "./build_ios.sh",
            args: &["--target", "sim"],
            // The script would also build the example app for a generic
            // simulator; the job builds it next for the chosen one.
            vars: &[("SKIP_EXAMPLE_APP_BUILD", "1")],
        },
        xcode_dir: "packages/swift-sdk",
        project: "SwiftExampleApp/SwiftExampleApp.xcodeproj",
        scheme: "SwiftExampleApp",
    }),
};

/// Dash Platform's Kotlin SDK and KotlinExampleApp
/// (`packages/kotlin-sdk/BUILD_GUIDE_FOR_AI.md`).
const PLATFORM_ANDROID: Recipe = Recipe {
    name: "KotlinExampleApp",
    markers: &[
        "packages/kotlin-sdk/build_android.sh",
        "packages/kotlin-sdk/gradlew",
        "packages/kotlin-sdk/KotlinExampleApp/app/build.gradle.kts",
    ],
    requires: &[
        CARGO,
        Tool {
            program: "cargo-ndk",
            missing: "cargo-ndk was not found. Install it with \"cargo install cargo-ndk\".",
        },
    ],
    kind: RecipeKind::Android(AndroidRecipe {
        sdk: Script {
            dir: "packages/kotlin-sdk",
            program: "./build_android.sh",
            args: &["--abi", "{abi}"],
            vars: &[],
        },
        abi_names: &[("arm64-v8a", "arm64"), ("x86_64", "x86_64")],
        gradle_dir: "packages/kotlin-sdk",
        // Gradle can otherwise package a stale native library ("Issue 1" in
        // the Kotlin build guide).
        gradle_tasks: &[":app:mergeDebugNativeLibs", ":app:installDebug"],
        app_build_file: "packages/kotlin-sdk/KotlinExampleApp/app/build.gradle.kts",
    }),
};

/// Every known recipe. The first whose markers exist wins, per platform.
pub const RECIPES: &[Recipe] = &[PLATFORM_IOS, PLATFORM_ANDROID];

/// Whether this machine can build `platform` at all.
pub fn platform_supported(platform: RunPlatform) -> bool {
    match platform {
        RunPlatform::Ios => cfg!(target_os = "macos"),
        RunPlatform::Android => true,
    }
}

/// The recipe for `platform` that applies to the tree at `root`.
pub fn find(root: &Path, platform: RunPlatform) -> Option<&'static Recipe> {
    RECIPES
        .iter()
        .find(|r| r.platform() == platform && r.applies_to(root))
}

/// The platforms with a recipe in the tree at `root`, iOS first, leaving out
/// those this machine cannot build.
pub fn platforms_in(root: &Path, ios_supported: bool) -> Vec<RunPlatform> {
    [RunPlatform::Ios, RunPlatform::Android]
        .into_iter()
        .filter(|p| *p != RunPlatform::Ios || ios_supported)
        .filter(|p| find(root, *p).is_some())
        .collect()
}

/// "iOS" or "Android", for sentences.
pub fn platform_name(platform: RunPlatform) -> &'static str {
    match platform {
        RunPlatform::Ios => "iOS",
        RunPlatform::Android => "Android",
    }
}

/// The `applicationId` in a Gradle Kotlin or Groovy build file.
pub fn application_id(build_file: &str) -> Option<String> {
    build_file.lines().find_map(|line| {
        let line = line.trim();
        let rest = line.strip_prefix("applicationId")?;
        let rest = rest.trim_start();
        let rest = rest.strip_prefix('=').unwrap_or(rest).trim_start();
        let quote = rest.chars().next().filter(|c| *c == '"' || *c == '\'')?;
        let value = rest[1..].split(quote).next()?;
        let usable = !value.is_empty()
            && value
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_');
        usable.then(|| value.to_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(root: &Path, rel: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, "").unwrap();
    }

    #[test]
    fn platforms_follow_the_markers() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert!(platforms_in(root, true).is_empty());
        for m in PLATFORM_ANDROID.markers {
            touch(root, m);
        }
        assert_eq!(platforms_in(root, true), vec![RunPlatform::Android]);
        touch(root, "packages/swift-sdk/build_ios.sh");
        // One marker is not enough.
        assert_eq!(platforms_in(root, true), vec![RunPlatform::Android]);
        std::fs::create_dir_all(
            root.join("packages/swift-sdk/SwiftExampleApp/SwiftExampleApp.xcodeproj"),
        )
        .unwrap();
        assert_eq!(
            platforms_in(root, true),
            vec![RunPlatform::Ios, RunPlatform::Android]
        );
        // On Linux iOS is never offered.
        assert_eq!(platforms_in(root, false), vec![RunPlatform::Android]);
        assert_eq!(find(root, RunPlatform::Ios), Some(&PLATFORM_IOS));
    }

    #[test]
    fn application_id_reads_kotlin_and_groovy() {
        let kts = r#"
android {
    namespace = "org.dashfoundation.example"
    defaultConfig {
        applicationId = "org.dashfoundation.example"
        minSdk = 29
"#;
        assert_eq!(
            application_id(kts).as_deref(),
            Some("org.dashfoundation.example")
        );
        assert_eq!(
            application_id("  applicationId 'com.example.app'\n").as_deref(),
            Some("com.example.app")
        );
        assert_eq!(application_id("applicationIdSuffix = \".debug\""), None);
        assert_eq!(application_id("applicationId = someVariable"), None);
        assert_eq!(application_id(""), None);
    }

    #[test]
    fn ios_is_only_supported_on_macos() {
        assert_eq!(
            platform_supported(RunPlatform::Ios),
            cfg!(target_os = "macos")
        );
        assert!(platform_supported(RunPlatform::Android));
    }
}
