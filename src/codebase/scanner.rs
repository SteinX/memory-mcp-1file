use std::path::{Path, PathBuf};

use ignore::{overrides::OverrideBuilder, WalkBuilder};

use crate::types::Language;

/// Directories that should be skipped entirely during traversal.
/// This prevents the walker from even descending into them (saves I/O).
const SKIP_DIRS: &[&str] = &[
    // Build artifacts
    "build",
    "dist",
    "out",
    "target",
    // JS/Node
    "node_modules",
    ".npm",
    ".yarn",
    ".pnp",
    // Dart/Flutter
    ".dart_tool",
    ".pub-cache",
    // Android/Gradle
    ".gradle",
    ".android",
    // iOS
    "Pods",
    ".symlinks",
    // Python
    "__pycache__",
    ".venv",
    "venv",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
    "egg-info",
    // Rust
    ".cargo",
    // IDE/Editor
    ".idea",
    ".vscode",
    ".vs",
    // Cache/temp
    // Coverage/test artifacts
    "coverage",
    ".nyc_output",
    // Container/infra
    ".terraform",
    // Generated
    ".generated",
    ".next",
    ".nuxt",
    ".svelte-kit",
];

pub fn scan_directory(root: &Path) -> crate::Result<Vec<PathBuf>> {
    let mut overrides = OverrideBuilder::new(root);
    // Generated dart files (match at any depth)
    let _ = overrides.add("!**/*.g.dart");
    let _ = overrides.add("!**/*.freezed.dart");
    let _ = overrides.add("!**/*.gr.dart");
    let _ = overrides.add("!**/*.config.dart");
    let _ = overrides.add("!**/*.mocks.dart");
    let _ = overrides.add("!**/*.arb");

    // Skip directories at any nesting depth via overrides
    for dir in SKIP_DIRS {
        let _ = overrides.add(&format!("!**/{dir}/**"));
    }

    let walker = WalkBuilder::new(root)
        .hidden(true)
        .git_ignore(true)
        .add_custom_ignore_filename(".memoryignore")
        .overrides(
            overrides
                .build()
                .unwrap_or_else(|_| ignore::overrides::Override::empty()),
        )
        .filter_entry(|entry| {
            // Early directory pruning — don't even descend into known-junk dirs.
            // This is the critical optimization: avoids stat()/readdir() on millions of files.
            if entry.file_type().is_some_and(|ft| ft.is_dir()) {
                if let Some(name) = entry.file_name().to_str() {
                    let name_lower = name.to_lowercase();
                    // Skip any directory whose name matches SKIP_DIRS
                    if SKIP_DIRS
                        .iter()
                        .any(|d| d.eq_ignore_ascii_case(&name_lower))
                    {
                        return false;
                    }
                    // Also skip hidden directories (start with .)
                    if name.starts_with('.') && name != "." && name != ".." {
                        return false;
                    }
                }
            }
            true
        })
        .build();

    let mut files = Vec::new();
    for entry in walker.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_file() && !is_ignored_file(path) && is_code_file(path) {
            files.push(path.to_path_buf());
        }
    }

    Ok(files)
}

pub fn is_ignored_file(path: &Path) -> bool {
    let path_str = path.to_string_lossy().to_lowercase();

    // Check if any path component matches a skip directory
    for dir in SKIP_DIRS {
        let pattern1 = format!("/{}/", dir);
        let pattern2 = format!("\\{}\\", dir);
        if path_str.contains(&pattern1) || path_str.contains(&pattern2) {
            return true;
        }
    }

    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_lowercase();

    // Hidden files (dotfiles)
    if name.starts_with('.') && name != "." {
        return true;
    }

    // Lock files
    if matches!(
        name.as_str(),
        "package-lock.json"
            | "yarn.lock"
            | "pnpm-lock.yaml"
            | "composer.lock"
            | "cargo.lock"
            | "pubspec.lock"
            | "gemfile.lock"
            | "poetry.lock"
    ) {
        return true;
    }

    // Generated / minified files
    name.ends_with(".g.dart")
        || name.ends_with(".freezed.dart")
        || name.ends_with(".gr.dart")
        || name.ends_with(".config.dart")
        || name.ends_with(".mocks.dart")
        || name.ends_with(".arb")
        || name.ends_with(".min.js")
        || name.ends_with(".min.css")
        || name.ends_with(".bundle.js")
        || name.ends_with(".map")
        || name.ends_with(".d.ts")
        || path_str.contains("/generated/")
        || path_str.contains("\\generated\\")
}

pub fn is_code_file(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };

    matches!(
        ext.to_lowercase().as_str(),
        "rs" | "py"
            | "js"
            | "ts"
            | "jsx"
            | "tsx"
            | "go"
            | "java"
            | "dart"
            | "c"
            | "cpp"
            | "cc"
            | "cxx"
            | "h"
            | "hpp"
            | "hh"
            | "hxx"
            | "m"
            | "mm"
            | "rb"
            | "php"
            | "swift"
            | "kt"
            | "kts"
            | "scala"
            | "sh"
            | "bash"
            | "zsh"
    )
}

pub fn detect_language(path: &Path) -> Language {
    detect_language_by_extension(path)
}

pub fn detect_language_with_content(path: &Path, content: &str) -> Language {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return Language::Unknown;
    };

    if ext.eq_ignore_ascii_case("h") {
        return detect_header_language(content);
    }

    detect_language_by_extension(path)
}

fn detect_language_by_extension(path: &Path) -> Language {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return Language::Unknown;
    };

    match ext.to_lowercase().as_str() {
        "rs" => Language::Rust,
        "py" => Language::Python,
        "js" | "jsx" => Language::JavaScript,
        "ts" | "tsx" => Language::TypeScript,
        "go" => Language::Go,
        "java" => Language::Java,
        "dart" => Language::Dart,
        "c" => Language::C,
        "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" => Language::Cpp,
        "swift" => Language::Swift,
        "kt" | "kts" => Language::Kotlin,
        "m" | "mm" => Language::ObjectiveC,
        _ => Language::Unknown,
    }
}

fn detect_header_language(content: &str) -> Language {
    let objc_markers = ["@interface", "@protocol", "@class", "@implementation"];
    if objc_markers.iter().any(|marker| content.contains(marker))
        || content.contains("#import <Foundation/")
        || content.contains("#import \"")
    {
        return Language::ObjectiveC;
    }

    let cpp_markers = [
        "namespace ",
        "template <",
        "class ",
        "std::",
        "public:",
        "private:",
        "protected:",
    ];
    if cpp_markers.iter().any(|marker| content.contains(marker)) {
        return Language::Cpp;
    }

    Language::C
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::fs;

    #[test]
    fn mobile_extension_contract() {
        let cases = [
            ("main.c", true, Language::C),
            ("main.cpp", true, Language::Cpp),
            ("main.cc", true, Language::Cpp),
            ("main.cxx", true, Language::Cpp),
            ("main.h", true, Language::Unknown),
            ("main.hpp", true, Language::Cpp),
            ("main.hh", true, Language::Cpp),
            ("main.hxx", true, Language::Cpp),
            ("main.swift", true, Language::Swift),
            ("main.kt", true, Language::Kotlin),
            ("main.kts", true, Language::Kotlin),
            ("main.m", true, Language::ObjectiveC),
            ("main.mm", true, Language::ObjectiveC),
            ("notes.txt", false, Language::Unknown),
        ];

        for (file_name, expected_code, expected_language) in cases {
            let path = Path::new(file_name);
            assert_eq!(is_code_file(path), expected_code, "{file_name}");
            assert_eq!(detect_language(path), expected_language, "{file_name}");
        }
    }

    #[test]
    fn header_heuristics_contract() {
        let objc_header = r#"
@interface ViewController : NSObject
- (void)loadData;
@end
"#;
        assert_eq!(
            detect_language_with_content(Path::new("ViewController.h"), objc_header),
            Language::ObjectiveC
        );

        let cpp_header = r#"
namespace demo {
template <typename T>
class Box {
public:
    T value;
};
}
"#;
        assert_eq!(
            detect_language_with_content(Path::new("box.h"), cpp_header),
            Language::Cpp
        );

        let c_header = r#"
typedef struct {
    int count;
} Counter;

void counter_init(Counter *counter);
"#;
        assert_eq!(
            detect_language_with_content(Path::new("counter.h"), c_header),
            Language::C
        );

        let ambiguous_header = r#"
struct Token {
    int id;
};

void token_init(struct Token *token);
"#;
        assert_eq!(
            detect_language_with_content(Path::new("token.h"), ambiguous_header),
            Language::C
        );
    }

    #[test]
    fn scanner_ignores_mobile_vendor_and_generated_dirs_regression() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let root = temp_dir.path();

        let keep_file = root.join("src").join("App.swift");
        let keep_header = root.join("include").join("Bridge.h");
        fs::create_dir_all(keep_file.parent().expect("parent")).expect("create src");
        fs::create_dir_all(keep_header.parent().expect("parent")).expect("create include");
        fs::write(&keep_file, "class App { func run() {} }").expect("write keep file");
        fs::write(&keep_header, "@interface Bridge : NSObject\n@end").expect("write keep header");

        let ignored_files = [
            root.join("Pods").join("Generated.m"),
            root.join(".gradle").join("cache").join("Build.kt"),
            root.join(".android").join("gen").join("MainActivity.kt"),
            root.join(".symlinks").join("plugins").join("Plugin.swift"),
            root.join("build").join("intermediates").join("Gen.cpp"),
            root.join(".generated").join("api").join("generated.mm"),
            root.join("generated").join("schema").join("generated.c"),
        ];

        for path in &ignored_files {
            fs::create_dir_all(path.parent().expect("parent")).expect("create ignored dir");
            fs::write(path, "fn ignored() {}").expect("write ignored file");
        }

        let scanned = scan_directory(root).expect("scan directory");
        let scanned_set: HashSet<String> = scanned
            .iter()
            .map(|path| {
                path.strip_prefix(root)
                    .expect("strip prefix")
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();

        assert!(scanned_set.contains("src/App.swift"));
        assert!(scanned_set.contains("include/Bridge.h"));

        for forbidden in ["Pods/", ".gradle/", ".android/", ".symlinks/", "build/"] {
            assert!(
                scanned_set.iter().all(|path| !path.starts_with(forbidden)),
                "scanner unexpectedly included file under {forbidden}: {:?}",
                scanned_set
            );
        }

        assert!(
            scanned_set.iter().all(|path| !path.contains("/generated/")),
            "scanner unexpectedly included generated directory file: {:?}",
            scanned_set
        );
        assert!(
            scanned_set
                .iter()
                .all(|path| !path.starts_with(".generated/")),
            "scanner unexpectedly included .generated directory file: {:?}",
            scanned_set
        );
    }
}
