//! Dependency extraction from subcommand failure output (R24).
//!
//! Parses tool-specific error messages to identify missing packages
//! and suggest install commands.

use crate::env::Detector;
use once_cell::sync::Lazy;
use regex::Regex;

static DEP_PATTERNS: Lazy<Vec<DepPattern>> = Lazy::new(|| {
    vec![
        // npm: "404 'package-name@version' is not in the npm registry"
        DepPattern {
            tool_name: "npm",
            re: Regex::new(r"npm ERR! 404\s+(?:Not Found\s+)?(?:'([^']+?)'|\S+@)").unwrap(),
            capture_group: 1,
            install_template: "npm install {pkg}",
        },
        // pip: "ERROR: No matching distribution found for package-name"
        DepPattern {
            tool_name: "pip",
            re: Regex::new(r"No matching distribution found for (\S+)").unwrap(),
            capture_group: 1,
            install_template: "pip install {pkg}",
        },
        // cargo: "could not find `crate_name` in registry"
        DepPattern {
            tool_name: "cargo",
            re: Regex::new(r"could not find `([^`]+)` in registry").unwrap(),
            capture_group: 1,
            install_template: "cargo add {pkg}",
        },
        // gem: "Could not find 'package-name' in any of the sources"
        DepPattern {
            tool_name: "gem",
            re: Regex::new(r"Could not find '([^']+)'").unwrap(),
            capture_group: 1,
            install_template: "gem install {pkg}",
        },
        // go: "cannot find package 'github.com/foo/bar'"
        DepPattern {
            tool_name: "go",
            re: Regex::new(r"cannot find package '([^']+)'").unwrap(),
            capture_group: 1,
            install_template: "go get {pkg}",
        },
        // docker: "manifest for image:tag not found"
        DepPattern {
            tool_name: "docker",
            re: Regex::new(r"manifest for (\S+) not found").unwrap(),
            capture_group: 1,
            install_template: "docker pull {pkg}",
        },
        // apt-get: "E: Unable to locate package X"
        DepPattern {
            tool_name: "apt",
            re: Regex::new(r"Unable to locate package (\S+)").unwrap(),
            capture_group: 1,
            install_template: "sudo apt update && sudo apt install {pkg}",
        },
        // composer: "Could not find package X"
        DepPattern {
            tool_name: "composer",
            re: Regex::new(r#"(?:Could not find|Root package) .*? requires? ['"]?([^'",]+)['"]?"#).unwrap(),
            capture_group: 1,
            install_template: "composer require {pkg}",
        },
        // pnpm: "ERR_PNPM_NO_PACKAGE_MANIFEST"
        DepPattern {
            tool_name: "pnpm",
            re: Regex::new(r"ERR_PNPM_\w+\s+(\S+)").unwrap(),
            capture_group: 1,
            install_template: "pnpm add {pkg}",
        },
        // yarn: "error Couldn't find package X"
        DepPattern {
            tool_name: "yarn",
            re: Regex::new(r"Couldn't find package (\S+)").unwrap(),
            capture_group: 1,
            install_template: "yarn add {pkg}",
        },
        // uv: "error: Package `X` not found"
        DepPattern {
            tool_name: "uv",
            re: Regex::new(r"Package [`']([^`']+)[`'] not found").unwrap(),
            capture_group: 1,
            install_template: "uv add {pkg}",
        },
        // poetry: "SolverProblemError because X is not found"
        DepPattern {
            tool_name: "poetry",
            re: Regex::new("(?:Package|')([^']+)[']? (?:is )?not found").unwrap(),
            capture_group: 1,
            install_template: "poetry add {pkg}",
        },
        // gradle: "Could not resolve dependency X"
        DepPattern {
            tool_name: "gradle",
            re: Regex::new(r"Could not resolve dependency\s+(\S+)").unwrap(),
            capture_group: 1,
            install_template: "gradle {pkg}",
        },
        // maven: "Could not resolve artifact X"
        DepPattern {
            tool_name: "mvn",
            re: Regex::new(r"Could not resolve artifact\s+(\S+)").unwrap(),
            capture_group: 1,
            install_template: "mvn install",
        },
        // bun: "error: Cannot find package X"
        DepPattern {
            tool_name: "bun",
            re: Regex::new(r"Cannot find package (\S+)").unwrap(),
            capture_group: 1,
            install_template: "bun add {pkg}",
        },
        // deno: "error: Cannot resolve dependency X"
        DepPattern {
            tool_name: "deno",
            re: Regex::new(r"resolve dependency\s+(\S+)").unwrap(),
            capture_group: 1,
            install_template: "deno add {pkg}",
        },
    ]
});

struct DepPattern {
    tool_name: &'static str,
    re: Regex,
    capture_group: usize,
    install_template: &'static str,
}

/// Attempt to extract a missing dependency name from tool-specific stderr.
/// Returns a suggested install command if one is found.
pub fn extract_missing_dep(stderr: &str, tool: &str) -> Option<String> {
    for pattern in DEP_PATTERNS.iter() {
        if !tool.starts_with(pattern.tool_name) {
            continue;
        }
        if let Some(caps) = pattern.re.captures(stderr) {
            if let Some(pkg) = caps.get(pattern.capture_group) {
                let pkg_name = pkg
                    .as_str()
                    .trim()
                    .trim_matches('\'')
                    .trim_matches('"');
                // Strip version specifiers for npm packages
                let pkg_name = pkg_name
                    .split('@')
                    .next()
                    .unwrap_or(pkg_name)
                    .trim()
                    .to_string();
                if !pkg_name.is_empty() {
                    return Some(pattern.install_template.replace("{pkg}", &pkg_name));
                }
            }
        }
    }
    None
}

/// Generate a recovery suggestion for R24 when a missing dependency is detected.
pub fn suggest_missing_dep(
    original_command: &str,
    stderr: &str,
    detector: &Detector,
) -> Option<String> {
    let tool = original_command.split_whitespace().next().unwrap_or("");
    let install_cmd = extract_missing_dep(stderr, tool)?;

    // Check if the package manager is available
    let pm = detector.package_manager.as_deref();
    let available = pm.is_none_or(|pm| detector.available_tools.iter().any(|t| t.name == pm));

    let confidence = if available {
        "Can run"
    } else {
        "May need to install package manager"
    };
    Some(format!(
        "Missing dependency detected in {} output. {}: `{}`\nReason: {}",
        tool,
        confidence,
        install_cmd,
        stderr.lines().next().unwrap_or("(no stderr)")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_npm_missing_package() {
        let stderr = "npm ERR! 404 'lodash' is not in the npm registry";
        let result = extract_missing_dep(stderr, "npm");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "npm install lodash");
    }

    #[test]
    fn extracts_pip_missing_package() {
        let stderr = "ERROR: No matching distribution found for requests";
        let result = extract_missing_dep(stderr, "pip");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "pip install requests");
    }

    #[test]
    fn extracts_cargo_missing_crate() {
        let stderr = "could not find `serde` in registry `crates-io`";
        let result = extract_missing_dep(stderr, "cargo");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "cargo add serde");
    }

    #[test]
    fn no_match_for_unknown_tool() {
        let stderr = "some random error";
        let result = extract_missing_dep(stderr, "unknown");
        assert!(result.is_none());
    }

    #[test]
    fn extracts_pnpm_missing() {
        let stderr = "ERR_PNPM_NO_PACKAGE_MANIFEST No package.json found";
        let result = extract_missing_dep(stderr, "pnpm");
        assert!(result.is_some());
    }

    #[test]
    fn extracts_yarn_missing() {
        let stderr = "error Couldn't find package lodash";
        let result = extract_missing_dep(stderr, "yarn");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "yarn add lodash");
    }

    #[test]
    fn extracts_uv_missing() {
        let stderr = "error: Package `requests` not found";
        let result = extract_missing_dep(stderr, "uv");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "uv add requests");
    }

    #[test]
    fn extracts_bun_missing() {
        let stderr = "error: Cannot find package express";
        let result = extract_missing_dep(stderr, "bun");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "bun add express");
    }

}

