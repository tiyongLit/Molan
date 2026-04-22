pub fn show_clean_help() -> String {
    [
        "Usage: mo clean [OPTIONS]",
        "",
        "Clean up disk space by removing caches, logs, temporary files, and app leftovers from already-uninstalled apps.",
        "",
        "Options:",
        "  --dry-run, -n     Preview cleanup without making changes",
        "  --external PATH   Clean OS metadata from a mounted external volume",
        "  --whitelist       Manage protected paths",
        "  --debug           Show detailed operation logs",
        "  -h, --help        Show this help message",
    ]
    .join("\n")
}

pub fn show_installer_help() -> String {
    [
        "Usage: mo installer [OPTIONS]",
        "",
        "Find and remove installer files (.dmg, .pkg, .iso, .xip, .zip).",
        "",
        "Options:",
        "  --dry-run         Preview installer cleanup without making changes",
        "  --debug           Show detailed operation logs",
        "  -h, --help        Show this help message",
    ]
    .join("\n")
}

pub fn show_optimize_help() -> String {
    [
        "Usage: mo optimize [OPTIONS]",
        "",
        "Check and maintain system health, apply optimizations.",
        "",
        "Options:",
        "  --dry-run         Preview optimization without making changes",
        "  --whitelist       Manage protected items",
        "  --debug           Show detailed operation logs",
        "  -h, --help        Show this help message",
    ]
    .join("\n")
}

pub fn show_touchid_help() -> String {
    [
        "Usage: mo touchid [COMMAND]",
        "",
        "Configure Touch ID for sudo authentication.",
        "",
        "Commands:",
        "  enable            Enable Touch ID for sudo",
        "  disable           Disable Touch ID for sudo",
        "  status            Show current Touch ID status",
        "",
        "Options:",
        "  --dry-run         Preview Touch ID changes without modifying sudo config",
        "  -h, --help        Show this help message",
        "",
        "If no command is provided, an interactive menu is shown.",
    ]
    .join("\n")
}

pub fn show_uninstall_help() -> String {
    [
        "Usage: mo uninstall [OPTIONS] [APP_NAME ...]",
        "",
        "Interactively remove applications and their leftover files.",
        "Optionally specify one or more app names to uninstall directly.",
        "For leftovers from apps that are already gone, use mo clean.",
        "",
        "Examples:",
        "  mo uninstall                   Open interactive app selector",
        "  mo uninstall slack             Uninstall Slack",
        "  mo uninstall slack zoom        Uninstall Slack and Zoom",
        "  mo uninstall --dry-run slack   Preview Slack uninstallation",
        "  mo uninstall --list            Show installed apps and the names mo uninstall accepts",
        "",
        "Options:",
        "  --list            List installed apps with the exact name mo uninstall accepts",
        "  --dry-run         Preview app uninstallation without making changes",
        "  --permanent       Bypass macOS Trash and rm -rf immediately",
        "  --whitelist       Not supported for uninstall (use clean/optimize)",
        "  --debug           Show detailed operation logs",
        "  -h, --help        Show this help message",
        "",
        "By default, uninstalled files go to the macOS Trash so they can be",
        "recovered. Use --permanent to skip the Trash step.",
    ]
    .join("\n")
}
