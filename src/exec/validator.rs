use once_cell::sync::Lazy;
use regex::Regex;

static DANGEROUS_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
        Regex::new(r"rm\s+-rf\s+/\s*($|\s)").unwrap(),
        Regex::new(r">\s*/dev/sda").unwrap(),
        Regex::new(r":\(\)\s*\{\s*:\|:\s*&\s*\};\s*:").unwrap(),
        Regex::new(r"mkfs\.").unwrap(),
        Regex::new(r"dd\s+if=.*of=/dev/[sh]d").unwrap(),
    ]
});

pub fn validate(command: &str) -> Result<(), String> {
    for re in DANGEROUS_PATTERNS.iter() {
        if re.is_match(command) {
            return Err(format!(
                "Dangerous command blocked by validator: pattern matched `{}`",
                command
            ));
        }
    }

    let interactive_commands = ["vim", "vi", "nano", "emacs", "less", "more", "man", "top", "htop"];
    let first_word = command.split_whitespace().next().unwrap_or("").trim();
    if interactive_commands.contains(&first_word) {
        return Err(format!(
            "Interactive command '{}' is blocked. Use non-interactive alternatives (e.g., cat, sed, grep).",
            first_word
        ));
    }

    // Simple unmatched quote check
    let mut in_single = false;
    let mut in_double = false;
    for c in command.chars() {
        match c {
            '\\' => {}
            '"' if !in_single => in_double = !in_double,
            '\'' if !in_double => in_single = !in_single,
            _ => {}
        }
    }
    if in_single || in_double {
        return Err("Unmatched quotes detected in command".to_string());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dangerous_blocked() {
        assert!(validate("rm -rf /").is_err());
        assert!(validate("echo > /dev/sda").is_err());
    }

    #[test]
    fn test_interactive_blocked() {
        assert!(validate("vim file.txt").is_err());
        assert!(validate("nano config").is_err());
    }

    #[test]
    fn test_quotes_unmatched() {
        assert!(validate("echo 'hello").is_err());
        assert!(validate("echo \"world").is_err());
    }

    #[test]
    fn test_valid_command() {
        assert!(validate("ls -la").is_ok());
        assert!(validate("echo 'hello world'").is_ok());
    }
}
