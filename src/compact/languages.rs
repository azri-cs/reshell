//! Language-aware skeleton extraction for output compaction.
//!
//! Detects programming languages by file extension or content patterns
//! and applies language-specific regex patterns to extract structural lines.

use once_cell::sync::Lazy;
use regex::Regex;

pub struct LanguageSkeleton {
    pub name: &'static str,
    pub patterns: Regex,
    pub extensions: &'static [&'static str],
    pub shebangs: &'static [&'static str],
}

static LANGUAGES: Lazy<Vec<LanguageSkeleton>> = Lazy::new(|| {
    vec![
        LanguageSkeleton {
            name: "rust",
            patterns: Regex::new(r"(?m)^\s*(pub\s+)?(fn |struct |enum |impl |trait |mod |macro_rules!|use |type |const |static |async fn )").unwrap(),
            extensions: &["rs"],
            shebangs: &[],
        },
        LanguageSkeleton {
            name: "python",
            patterns: Regex::new(r"(?m)^\s*(def |class |async def |import |from |@)").unwrap(),
            extensions: &["py", "pyi", "pyx"],
            shebangs: &["python", "python3"],
        },
        LanguageSkeleton {
            name: "go",
            patterns: Regex::new(r"(?m)^\s*(func |type |var |const |package |import |interface |struct )").unwrap(),
            extensions: &["go"],
            shebangs: &[],
        },
        LanguageSkeleton {
            name: "javascript",
            patterns: Regex::new(r"(?m)^\s*(function |class |export |import |const |let |var |interface |type |async function )").unwrap(),
            extensions: &["js", "jsx", "ts", "tsx", "mjs", "cjs"],
            shebangs: &["node"],
        },
        LanguageSkeleton {
            name: "java",
            patterns: Regex::new(r"(?m)^\s*(public |private |protected |class |interface |enum |@Override|@Test)").unwrap(),
            extensions: &["java"],
            shebangs: &[],
        },
        LanguageSkeleton {
            name: "terraform",
            patterns: Regex::new(r#"(?m)^\s*(resource|module|variable|output|provider|data|terraform)\s+\""#).unwrap(),
            extensions: &["tf", "tfvars"],
            shebangs: &[],
        },
        LanguageSkeleton {
            name: "sql",
            patterns: Regex::new(r"(?mi)^\s*(CREATE|ALTER|DROP|INSERT|SELECT|UPDATE|DELETE|GRANT|REVOKE)\s").unwrap(),
            extensions: &["sql"],
            shebangs: &[],
        },
        LanguageSkeleton {
            name: "dockerfile",
            patterns: Regex::new(r"(?mi)^\s*(FROM|RUN|COPY|ADD|EXPOSE|ENV|CMD|ENTRYPOINT|VOLUME|WORKDIR|USER)\s").unwrap(),
            extensions: &["Dockerfile"],
            shebangs: &[],
        },
        LanguageSkeleton {
            name: "makefile",
            patterns: Regex::new(r"(?m)^[^\s#][^:]*:").unwrap(),
            extensions: &["Makefile", "mk"],
            shebangs: &[],
        },
        LanguageSkeleton {
            name: "toml",
            patterns: Regex::new(r"(?m)^\[").unwrap(),
            extensions: &["toml"],
            shebangs: &[],
        },
        LanguageSkeleton {
            name: "yaml",
            patterns: Regex::new(r"(?m)^\S+:").unwrap(),
            extensions: &["yaml", "yml"],
            shebangs: &[],
        },
        LanguageSkeleton {
            name: "shell",
            patterns: Regex::new(r"(?m)^\s*(function |if |for |while |case |alias |export )").unwrap(),
            extensions: &["sh", "bash", "zsh"],
            shebangs: &["sh", "bash", "zsh", "dash"],
        },
    ]
});

/// Generic fallback skeleton regex (used when no language is detected).
static GENERIC_SKELETON_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?m)^\s*(fn |function |class |struct |mod |pub fn |impl |ERROR|WARN|INFO|DEBUG|TRACE|FATAL)").unwrap()
});

/// Detect language by filename extension.
pub fn detect_by_extension(filename: Option<&str>) -> Option<&'static LanguageSkeleton> {
    let filename = filename?;
    let ext = filename.rsplit('.').next()?;
    LANGUAGES.iter().find(|l| l.extensions.contains(&ext))
}

/// Detect language by content (shebang line).
pub fn detect_by_content(content: &str) -> Option<&'static LanguageSkeleton> {
    let first_line = content.lines().next()?;
    if let Some(shebang) = first_line.strip_prefix("#!") {
        let shebang = shebang.trim();
        // Split into words: e.g., "/usr/bin/env python3" -> ["/usr/bin/env", "python3"]
        let mut words = shebang.split_whitespace();
        // If the first word ends with "env", the second word is the program
        let prog = if words
            .clone()
            .next()
            .map(|w| w.ends_with("env"))
            .unwrap_or(false)
        {
            words.nth(1).unwrap_or("")
        } else {
            words.next().unwrap_or("")
        };
        let prog = prog.rsplit('/').next().unwrap_or(prog);
        LANGUAGES.iter().find(|l| l.shebangs.contains(&prog))
    } else {
        None
    }
}

/// Detect the best-matching language for given content and optional filename.
/// Filename takes priority, then content shebang, then returns None (use generic).
pub fn detect_language(content: &str, filename: Option<&str>) -> Option<&'static LanguageSkeleton> {
    detect_by_extension(filename).or_else(|| detect_by_content(content))
}

/// Extract skeleton lines using the best-matching language pattern.
/// Falls back to generic patterns if no language is detected.
pub fn extract_skeleton_for_language(content: &str, language: Option<&LanguageSkeleton>) -> String {
    let re = language
        .map(|l| &l.patterns)
        .unwrap_or(&GENERIC_SKELETON_RE);

    let mut lines: Vec<&str> = content
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty() && re.is_match(trimmed)
        })
        .collect();

    // Deduplicate while preserving order
    let mut seen = std::collections::HashSet::new();
    lines.retain(|&line| seen.insert(line));

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_rust_by_extension() {
        let lang = detect_by_extension(Some("main.rs"));
        assert!(lang.is_some());
        assert_eq!(lang.unwrap().name, "rust");
    }

    #[test]
    fn detects_python_by_shebang() {
        let content = "#!/usr/bin/env python3\ndef main(): pass";
        let lang = detect_by_content(content);
        assert!(lang.is_some());
        assert_eq!(lang.unwrap().name, "python");
    }

    #[test]
    fn detects_go_by_extension() {
        let lang = detect_by_extension(Some("server.go"));
        assert!(lang.is_some());
        assert_eq!(lang.unwrap().name, "go");
    }

    #[test]
    fn extracts_rust_skeleton() {
        let content = "fn main() { println!(\"hi\"); }\npub struct Foo;\nlet x = 1;";
        let lang = detect_by_extension(Some("main.rs"));
        let skel = extract_skeleton_for_language(content, lang);
        assert!(skel.contains("fn main"));
        assert!(skel.contains("pub struct Foo"));
        assert!(!skel.contains("let x"));
    }

    #[test]
    fn extracts_python_skeleton() {
        let content = "def foo():\n    pass\n\nclass Bar:\n    pass\n\nprint('hi')";
        let lang = detect_by_extension(Some("script.py"));
        let skel = extract_skeleton_for_language(content, lang);
        assert!(skel.contains("def foo"));
        assert!(skel.contains("class Bar"));
        assert!(!skel.contains("print"));
    }

    #[test]
    fn unknown_extension_uses_generic() {
        let content = "fn hello() {}\nclass Foo {}\nERROR something\nplain line";
        let skel = extract_skeleton_for_language(content, None);
        assert!(skel.contains("fn hello"));
        assert!(skel.contains("class Foo"));
        assert!(skel.contains("ERROR something"));
        assert!(!skel.contains("plain line"));
    }
}
