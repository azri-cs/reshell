pub fn is_macos() -> bool {
    std::env::consts::OS == "macos"
}

pub fn is_linux() -> bool {
    std::env::consts::OS == "linux"
}

pub fn is_windows() -> bool {
    std::env::consts::OS == "windows"
}
