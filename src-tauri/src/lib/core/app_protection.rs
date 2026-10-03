use std::path::Path;
use std::process::Command;
use std::sync::{Mutex, OnceLock};

use super::base::{extract_bundle_from_path, normalize_slashes, pgrep_x, wildcard_match};
use rayon::prelude::*;

// 全局白名单存储:对齐 SH 中 WHITELIST_PATTERNS 全局数组
// GUI/CLI 在启动时通过 set_global_whitelist 注入,然后所有 safe_* 操作就会自动尊重它
static GLOBAL_WHITELIST: OnceLock<Mutex<Vec<String>>> = OnceLock::new();

pub fn set_global_whitelist(patterns: Vec<String>) {
    let cell = GLOBAL_WHITELIST.get_or_init(|| Mutex::new(Vec::new()));
    if let Ok(mut g) = cell.lock() {
        *g = patterns;
    }
}

pub fn get_global_whitelist() -> Vec<String> {
    let cell = GLOBAL_WHITELIST.get_or_init(|| Mutex::new(Vec::new()));
    cell.lock().map(|g| g.clone()).unwrap_or_default()
}

static SYSTEM_CRITICAL_BUNDLES: &[&str] = &[
    "com.apple.finder",
    "com.apple.dock",
    "com.apple.Safari",
    "com.apple.mail",
    "com.apple.systempreferences",
    "com.apple.SystemSettings",
    "com.apple.Settings*",
    "com.apple.controlcenter*",
    "com.apple.Spotlight",
    "com.apple.notificationcenterui",
    "com.apple.loginwindow",
    "com.apple.Preview",
    "com.apple.TextEdit",
    "com.apple.Notes",
    "com.apple.reminders",
    "com.apple.iCal",
    "com.apple.AddressBook",
    "com.apple.Photos",
    "com.apple.AppStore",
    "com.apple.calculator",
    "com.apple.Dictionary",
    "com.apple.ScreenSharing",
    "com.apple.ActivityMonitor",
    "com.apple.Console",
    "com.apple.DiskUtility",
    "com.apple.KeychainAccess",
    "com.apple.DigitalColorMeter",
    "com.apple.grapher",
    "com.apple.Terminal",
    "com.apple.ScriptEditor2",
    "com.apple.VoiceOverUtility",
    "com.apple.BluetoothFileExchange",
    "com.apple.print.PrinterProxy",
    "com.apple.systempreferences*",
    "com.apple.SystemProfiler",
    "com.apple.FontBook",
    "com.apple.ColorSyncUtility",
    "com.apple.audio.AudioMIDISetup",
    "com.apple.DirectoryUtility",
    "com.apple.NetworkUtility",
    "com.apple.exposelauncher",
    "com.apple.MigrateAssistant",
    "com.apple.RAIDUtility",
    "com.apple.BootCampAssistant",
    "com.apple.SecurityAgent",
    "com.apple.CoreServices*",
    "com.apple.SystemUIServer",
    "com.apple.backgroundtaskmanagement*",
    "com.apple.loginitems*",
    "com.apple.sharedfilelist*",
    "com.apple.sfl*",
    "com.apple.coreservices*",
    "com.apple.metadata*",
    "com.apple.MobileSoftwareUpdate*",
    "com.apple.SoftwareUpdate*",
    "com.apple.installer*",
    "com.apple.frameworks*",
    "com.apple.security*",
    "com.apple.keychain*",
    "com.apple.trustd*",
    "com.apple.securityd*",
    "com.apple.cloudd*",
    "com.apple.iCloud*",
    "com.apple.WiFi*",
    "com.apple.airport*",
    "com.apple.Bluetooth*",
    "com.apple.inputmethod.*",
    "com.apple.inputsource*",
    "com.apple.TextInput*",
    "com.apple.CharacterPicker*",
    "com.apple.PressAndHold*",
    "loginwindow",
    "dock",
    "systempreferences",
    "finder",
    "safari",
    "backgroundtaskmanagementagent",
    "keychain*",
    "security*",
    "bluetooth*",
    "wifi*",
    "network*",
    "tcc",
    "notification*",
    "accessibility*",
    "universalaccess*",
    "HIToolbox*",
    "textinput*",
    "TextInput*",
    "keyboard*",
    "Keyboard*",
    "inputsource*",
    "InputSource*",
    "keylayout*",
    "KeyLayout*",
    "GlobalPreferences",
    ".GlobalPreferences",
    "org.pqrs.Karabiner*",
];

static SYSTEM_CRITICAL_BUNDLES_FAST: &[&str] = &[
    "com.apple.*",
    "loginwindow",
    "dock",
    "systempreferences",
    "finder",
    "safari",
    "backgroundtaskmanagement*",
    "keychain*",
    "security*",
    "bluetooth*",
    "wifi*",
    "network*",
    "tcc",
    "notification*",
    "accessibility*",
    "universalaccess*",
    "HIToolbox*",
    "textinput*",
    "TextInput*",
    "keyboard*",
    "Keyboard*",
    "inputsource*",
    "InputSource*",
    "keylayout*",
    "KeyLayout*",
    "GlobalPreferences",
    ".GlobalPreferences",
    "org.pqrs.Karabiner*",
    "org.cups.*",
];

static APPLE_UNINSTALLABLE_APPS: &[&str] = &[
    "com.apple.dt.*",
    "com.apple.FinalCut*",
    "com.apple.Motion",
    "com.apple.Compressor",
    "com.apple.logic*",
    "com.apple.garageband*",
    "com.apple.iMovie",
    "com.apple.iWork.*",
    "com.apple.MainStage*",
    "com.apple.server.*",
    "com.apple.Playgrounds",
];

static DATA_PROTECTED_BUNDLES: &[&str] = &[
    "com.tencent.inputmethod.QQInput",
    "com.sogou.inputmethod.*",
    "com.baidu.inputmethod.*",
    "com.googlecode.rimeime.*",
    "im.rime.*",
    "*.inputmethod",
    "*.InputMethod",
    "*IME",
    "com.nektony.*",
    "com.macpaw.*",
    "com.freemacsoft.AppCleaner",
    "com.omnigroup.omnidisksweeper",
    "com.daisydiskapp.*",
    "com.tunabellysoftware.*",
    "com.grandperspectiv.*",
    "com.binaryfruit.*",
    "com.1password.*",
    "com.agilebits.*",
    "com.lastpass.*",
    "com.dashlane.*",
    "com.bitwarden.*",
    "com.keepassx.*",
    "org.keepassx.*",
    "org.keepassxc.*",
    "com.authy.*",
    "com.yubico.*",
    "com.jetbrains.*",
    "JetBrains*",
    "com.microsoft.VSCode",
    "com.visualstudio.code.*",
    "com.sublimetext.*",
    "com.sublimehq.*",
    "com.microsoft.VSCodeInsiders",
    "com.apple.dt.Xcode",
    "com.coteditor.CotEditor",
    "com.macromates.TextMate",
    "com.panic.Nova",
    "abnerworks.Typora",
    "com.uranusjr.macdown",
    "com.todesktop.*",
    "Cursor",
    "com.anthropic.claude*",
    "Claude",
    "com.openai.chat*",
    "ChatGPT",
    "com.ollama.ollama",
    "Ollama",
    "com.lmstudio.lmstudio",
    "LM Studio",
    "co.supertool.chatbox",
    "page.jan.jan",
    "com.huggingface.huggingchat",
    "Gemini",
    "com.perplexity.Perplexity",
    "com.drawthings.DrawThings",
    "com.divamgupta.diffusionbee",
    "com.exafunction.windsurf",
    "com.quora.poe.electron",
    "chat.openai.com.*",
    "com.sequelpro.*",
    "com.sequel-ace.*",
    "com.tinyapp.*",
    "com.dbeaver.*",
    "com.navicat.*",
    "com.mongodb.compass",
    "com.redis.RedisInsight",
    "com.pgadmin.pgadmin4",
    "com.eggerapps.Sequel-Pro",
    "com.valentina-db.Valentina-Studio",
    "com.dbvis.DbVisualizer",
    "com.postmanlabs.mac",
    "com.konghq.insomnia",
    "com.CharlesProxy.*",
    "com.proxyman.*",
    "com.getpaw.*",
    "com.luckymarmot.Paw",
    "com.charlesproxy.charles",
    "com.telerik.Fiddler",
    "com.usebruno.app",
    "com.clash.*",
    "ClashX*",
    "clash-*",
    "Clash-*",
    "*-clash",
    "*-Clash",
    "clash.*",
    "Clash.*",
    "clash_*",
    "*clash-verge*",
    "*Clash-Verge*",
    "clashverge*",
    "ClashVerge*",
    "com.nssurge.surge-mac",
    "*surge*",
    "*Surge*",
    "mihomo*",
    "*openvpn*",
    "*OpenVPN*",
    "net.openvpn.*",
    "*ShadowsocksX-NG*",
    "com.qiuyuzhou.*",
    "*v2ray*",
    "*V2Ray*",
    "*v2box*",
    "*V2Box*",
    "*nekoray*",
    "*sing-box*",
    "*OneBox*",
    "*hiddify*",
    "*Hiddify*",
    "*loon*",
    "*Loon*",
    "*quantumult*",
    "*tailscale*",
    "io.tailscale.*",
    "*zerotier*",
    "com.zerotier.*",
    "*1dot1dot1dot1*",
    "*cloudflare*warp*",
    "*nordvpn*",
    "*expressvpn*",
    "*protonvpn*",
    "*surfshark*",
    "*windscribe*",
    "*mullvad*",
    "*privateinternetaccess*",
    "*Aerial.saver*",
    "com.JohnCoates.Aerial*",
    "*Fliqlo*",
    "*fliqlo*",
    "com.github.GitHubDesktop",
    "com.sublimemerge",
    "com.torusknot.SourceTreeNotMAS",
    "com.git-tower.Tower*",
    "com.gitfox.GitFox",
    "com.github.Gitify",
    "com.fork.Fork",
    "com.axosoft.gitkraken",
    "com.googlecode.iterm2",
    "net.kovidgoyal.kitty",
    "io.alacritty",
    "com.github.wez.wezterm",
    "com.hyper.Hyper",
    "com.mizage.divvy",
    "com.fig.Fig",
    "dev.warp.Warp-Stable",
    "com.termius-dmg",
    "com.docker.docker",
    "com.getutm.UTM",
    "com.vmware.fusion",
    "com.parallels.desktop.*",
    "org.virtualbox.app.VirtualBox",
    "com.vagrant.*",
    "com.orbstack.OrbStack",
    "com.bjango.istatmenus*",
    "eu.exelban.Stats",
    "com.monitorcontrol.*",
    "com.bresink.system-toolkit.*",
    "com.mediaatelier.MenuMeters",
    "com.activity-indicator.app",
    "net.cindori.sensei",
    // Window Management
    "com.macitbetter.*",
    "com.hegenberg.*",
    "com.manytricks.*",
    "com.divisiblebyzero.*",
    "com.koingdev.*",
    "com.if.Amphetamine",
    "com.lwouis.alt-tab-macos",
    "net.matthewpalmer.Vanilla",
    "com.lightheadsw.Caffeine",
    "com.contextual.Contexts",
    "com.amethyst.Amethyst",
    "com.knollsoft.Rectangle",
    "com.knollsoft.Hookshot",
    "com.surteesstudios.Bartender",
    "com.gaosun.eul",
    "com.pointum.hazeover",
    // Launcher & Automation
    "com.runningwithcrayons.Alfred",
    "com.raycast.macos",
    "com.blacktree.Quicksilver",
    "com.stairways.keyboardmaestro.*",
    "com.manytricks.Butler",
    "com.happenapps.Quitter",
    "com.pilotmoon.scroll-reverser",
    "org.pqrs.Karabiner-Elements",
    "com.apple.Automator",
    // Note-Taking
    "com.bear-writer.*",
    "com.typora.*",
    "com.ulyssesapp.*",
    "com.literatureandlatte.*",
    "com.dayoneapp.*",
    "notion.id",
    "md.obsidian",
    "com.logseq.logseq",
    "com.evernote.Evernote",
    "com.onenote.mac",
    "com.omnigroup.OmniOutliner*",
    "net.shinyfrog.bear",
    "com.goodnotes.GoodNotes",
    "com.marginnote.MarginNote*",
    "com.roamresearch.*",
    "com.reflect.ReflectApp",
    "com.inkdrop.*",
    // Design & Creative
    "com.adobe.*",
    "com.bohemiancoding.*",
    "com.figma.*",
    "com.framerx.*",
    "com.zeplin.*",
    "com.invisionapp.*",
    "com.principle.*",
    "com.pixelmatorteam.*",
    "com.affinitydesigner.*",
    "com.affinityphoto.*",
    "com.affinitypublisher.*",
    "com.linearity.curve",
    "com.canva.CanvaDesktop",
    "com.maxon.cinema4d",
    "com.autodesk.*",
    "com.sketchup.*",
    // Communication
    "com.tencent.xinWeChat",
    "com.tencent.qq",
    "com.alibaba.DingTalkMac",
    "com.alibaba.AliLang.osx",
    "com.alibaba.alilang3.osx.ShipIt",
    "com.alibaba.AlilangMgr.QueryNetworkInfo",
    "us.zoom.xos",
    "com.microsoft.teams*",
    "com.slack.Slack",
    "com.hnc.Discord",
    "app.legcord.Legcord",
    "org.telegram.desktop",
    "ru.keepcoder.Telegram",
    "net.whatsapp.WhatsApp",
    "com.skype.skype",
    "com.cisco.webexmeetings",
    "com.ringcentral.RingCentral",
    "com.readdle.smartemail-Mac",
    "com.airmail.*",
    "com.postbox-inc.postbox",
    "com.tinyspeck.slackmacgap",
    // Task Management
    "com.omnigroup.OmniFocus*",
    "com.culturedcode.*",
    "com.todoist.*",
    "com.any.do.*",
    "com.ticktick.*",
    "com.microsoft.to-do",
    "com.trello.trello",
    "com.asana.nativeapp",
    "com.clickup.*",
    "com.monday.desktop",
    "com.airtable.airtable",
    "com.notion.id",
    "com.linear.linear",
    // File Transfer & Sync
    "com.panic.transmit*",
    "com.binarynights.ForkLift*",
    "com.noodlesoft.Hazel",
    "com.cyberduck.Cyberduck",
    "io.filezilla.FileZilla",
    "com.apple.Xcode.CloudDocuments",
    "com.synology.*",
    // Cloud Storage & Backup
    "com.getdropbox.*",
    "com.box.desktop*",
    "com.dropbox.*",
    "*dropbox*",
    "ws.agile.*",
    "com.backblaze.*",
    "*backblaze*",
    "*box.desktop*",
    "com.microsoft.SyncReporter",
    "com.microsoft.OneDrive*",
    "*OneDrive*",
    "com.google.GoogleDrive",
    "com.google.keystone*",
    "*GoogleDrive*",
    "com.amazon.drive",
    "com.apple.bird",
    "com.apple.CloudDocs*",
    "com.displaylink.*",
    "com.fujitsu.pfu.ScanSnap*",
    "com.citrix.*",
    "org.xquartz.*",
    "us.zoom.updater*",
    "com.DigiDNA.iMazing*",
    "com.shirtpocket.*",
    "homebrew.mxcl.*",
    // Screenshot & Recording
    "com.cleanshot.*",
    "com.xnipapp.xnip",
    "com.reincubate.camo",
    "com.tunabellysoftware.ScreenFloat",
    "net.telestream.screenflow*",
    "com.techsmith.snagit*",
    "com.techsmith.camtasia*",
    "com.obsidianapp.screenrecorder",
    "com.kap.Kap",
    "com.getkap.*",
    "com.linebreak.CloudApp",
    "com.droplr.droplr-mac",
    // Media & Entertainment
    "com.spotify.client",
    "org.mozilla.*",
    "Firefox",
    "org.videolan.vlc",
    "com.colliderli.iina",
    "com.apple.Music",
    "com.apple.podcasts",
    "com.apple.BKAgentService",
    "com.apple.iBooksX",
    "com.apple.iBooks",
    "com.blackmagic-design.*",
    "io.mpv",
    "tv.plex.player.desktop",
    "com.netease.163music",
    // Scientific & Professional
    "com.sas.*",
    "com.mathworks.*",
    "com.ibm.spss.*",
    "com.wolfram.*",
    "com.stata.*",
    "org.rstudio.*",
    "com.tableausoftware.*",
    // License & App Stores
    "com.paddle.Paddle*",
    "com.setapp.DesktopClient",
    "com.devmate.*",
    "org.sparkle-project.Sparkle",
];

static APPLE_UNINSTALLABLE_REGEX: OnceLock<String> = OnceLock::new();
static SYSTEM_CRITICAL_REGEX: OnceLock<String> = OnceLock::new();
static SYSTEM_CRITICAL_FAST_REGEX: OnceLock<String> = OnceLock::new();
static DATA_PROTECTED_REGEX: OnceLock<String> = OnceLock::new();

pub fn is_critical_system_component(bundle_id: &str) -> bool {
    if bundle_id.is_empty() {
        return false;
    }
    let lower = bundle_id.to_ascii_lowercase();
    lower.contains("backgroundtaskmanagement")
        || lower.contains("loginitems")
        || lower.contains("systempreferences")
        || lower.contains("systemsettings")
        || lower.contains("settings")
        || lower.contains("preferences")
        || lower.contains("controlcenter")
        || lower.contains("biometrickit")
        || lower.contains("sfl")
        || lower.contains("tcc")
}

pub fn bundle_matches_pattern(bundle_id: &str, pattern: &str) -> bool {
    wildcard_match(bundle_id, pattern)
}

pub fn build_regex_var(patterns: &[String]) -> String {
    patterns
        .iter()
        .map(|p| format!("^{}$", p.replace('.', "\\.").replace('*', ".*")))
        .collect::<Vec<_>>()
        .join("|")
}

pub fn _ensure_uninstall_regex() -> bool {
    let uninstallable: Vec<String> = APPLE_UNINSTALLABLE_APPS
        .iter()
        .map(|s| s.to_string())
        .collect();
    let critical: Vec<String> = SYSTEM_CRITICAL_BUNDLES
        .iter()
        .map(|s| s.to_string())
        .collect();
    let _ = APPLE_UNINSTALLABLE_REGEX.get_or_init(|| build_regex_var(&uninstallable));
    let _ = SYSTEM_CRITICAL_REGEX.get_or_init(|| build_regex_var(&critical));
    true
}

/// 对齐 bin/clean.sh:PROTECTED_SW_DOMAINS 第 32-58 行。
pub const PROTECTED_SW_DOMAINS: &[&str] = &[
    "capcut.com",
    "photopea.com",
    "pixlr.com",
    "docs.google.com",
    "sheets.google.com",
    "slides.google.com",
    "drive.google.com",
    "mail.google.com",
    "github.com",
    "gitlab.com",
    "codepen.io",
    "codesandbox.io",
    "replit.com",
    "stackblitz.com",
    "notion.so",
    "figma.com",
    "linear.app",
    "excalidraw.com",
];

/// 便捷封装：从全局白名单取模式后调用 `is_path_whitelisted`。
pub fn is_path_whitelisted_from_global(path: &str) -> bool {
    let patterns = get_global_whitelist();
    !patterns.is_empty() && is_path_whitelisted(path, &patterns)
}

pub fn _ensure_data_protection_regex() -> bool {
    let critical_fast: Vec<String> = SYSTEM_CRITICAL_BUNDLES_FAST
        .iter()
        .map(|s| s.to_string())
        .collect();
    let data_protected: Vec<String> = DATA_PROTECTED_BUNDLES
        .iter()
        .map(|s| s.to_string())
        .collect();
    let _ = SYSTEM_CRITICAL_FAST_REGEX.get_or_init(|| build_regex_var(&critical_fast));
    let _ = DATA_PROTECTED_REGEX.get_or_init(|| build_regex_var(&data_protected));
    true
}

pub fn should_protect_from_uninstall(bundle_id: &str) -> bool {
    _ensure_uninstall_regex();
    if APPLE_UNINSTALLABLE_APPS
        .iter()
        .any(|p| wildcard_match(bundle_id, p))
    {
        return false;
    }
    SYSTEM_CRITICAL_BUNDLES
        .iter()
        .any(|p| wildcard_match(bundle_id, p))
}

pub fn should_protect_data(bundle_id: &str) -> bool {
    _ensure_data_protection_regex();
    if bundle_id.is_empty() {
        return false;
    }

    // Fast path: 命中 SH should_protect_data() 早期 case 分支
    // 对应 app_protection.sh 第 686-742 行
    if bundle_id.starts_with("com.apple.")
        || bundle_id == "loginwindow"
        || bundle_id == "dock"
        || bundle_id == "systempreferences"
        || bundle_id == "finder"
        || bundle_id == "safari"
    {
        return true;
    }
    if bundle_id.starts_with("org.cups.") {
        return true;
    }
    if bundle_id.starts_with("backgroundtaskmanagement")
        || bundle_id.starts_with("keychain")
        || bundle_id.starts_with("security")
        || bundle_id.starts_with("bluetooth")
        || bundle_id.starts_with("wifi")
        || bundle_id.starts_with("network")
        || bundle_id == "tcc"
    {
        return true;
    }
    if bundle_id.starts_with("notification")
        || bundle_id.starts_with("accessibility")
        || bundle_id.starts_with("universalaccess")
        || bundle_id.starts_with("HIToolbox")
    {
        return true;
    }
    if bundle_id.contains("inputmethod")
        || bundle_id.contains("InputMethod")
        || bundle_id.ends_with("IME")
        || bundle_id.starts_with("textinput")
        || bundle_id.starts_with("TextInput")
    {
        return true;
    }
    if bundle_id.starts_with("keyboard")
        || bundle_id.starts_with("Keyboard")
        || bundle_id.starts_with("inputsource")
        || bundle_id.starts_with("InputSource")
        || bundle_id.starts_with("keylayout")
        || bundle_id.starts_with("KeyLayout")
    {
        return true;
    }
    if bundle_id == "GlobalPreferences"
        || bundle_id == ".GlobalPreferences"
        || bundle_id.starts_with("org.pqrs.Karabiner")
    {
        return true;
    }
    if bundle_id.starts_with("com.1password.")
        || bundle_id.starts_with("com.agilebits.")
        || bundle_id.starts_with("com.lastpass.")
        || bundle_id.starts_with("com.dashlane.")
        || bundle_id.starts_with("com.bitwarden.")
    {
        return true;
    }
    // 重要:对齐 SH 中宽口径分支
    if bundle_id.starts_with("com.jetbrains.")
        || bundle_id.starts_with("JetBrains")
        || bundle_id.starts_with("com.microsoft.")
        || bundle_id.starts_with("com.visualstudio.")
    {
        return true;
    }
    if bundle_id.starts_with("com.sublimetext.")
        || bundle_id.starts_with("com.sublimehq.")
        || bundle_id == "Cursor"
        || bundle_id == "Claude"
        || bundle_id == "ChatGPT"
        || bundle_id == "Ollama"
    {
        return true;
    }
    if bundle_id.starts_with("com.docker.")
        || bundle_id.starts_with("com.getpostman.")
        || bundle_id.starts_with("com.insomnia.")
    {
        return true;
    }

    // Fallback: 精确表匹配
    if SYSTEM_CRITICAL_BUNDLES_FAST
        .iter()
        .any(|p| wildcard_match(bundle_id, p))
    {
        return true;
    }
    DATA_PROTECTED_BUNDLES
        .iter()
        .any(|p| wildcard_match(bundle_id, p))
}

pub fn should_protect_path(path: &str) -> bool {
    if path.is_empty() {
        return false;
    }

    // Container 内 Cache/tmp 通路:对齐 app_protection.sh 第 813 行
    // 这些目录是系统/应用可重建的,即便 bundle id 是受保护的也应允许清理
    let mut container_cache_path = false;

    // 1. 关键字匹配(case-insensitive,对齐 SH 第 769-779 行 *[Ss]ystem[Ss]ettings* 等)
    let lower = path.to_ascii_lowercase();
    if lower.contains("systemsettings")
        || lower.contains("systempreferences")
        || lower.contains("controlcenter")
        || lower.contains("com.apple.settings")
        || lower.contains("com.apple.notes")
    {
        return true;
    }

    // 2. 关键 cache 文件保护(对齐 SH 第 783-803 行)
    if lower.contains("com.apple.systempreferences.cache")
        || lower.contains("com.apple.settings.cache")
        || lower.contains("com.apple.controlcenter.cache")
        || lower.contains("com.apple.finder.cache")
        || lower.contains("com.apple.dock.cache")
    {
        return true;
    }
    if path.contains("/Library/Containers/com.apple.Settings")
        || path.contains("/Library/Containers/com.apple.SystemSettings")
        || path.contains("/Library/Containers/com.apple.controlcenter")
        || path.contains("/Library/Group Containers/com.apple.systempreferences")
        || path.contains("/Library/Group Containers/com.apple.Settings")
    {
        return true;
    }
    // sharedfilelist 系统设置共享列表(macOS Sequoia)
    if path.contains("/com.apple.sharedfilelist/") {
        let lname = path.to_ascii_lowercase();
        if lname.contains("com.apple.settings")
            || lname.contains("com.apple.systemsettings")
            || lname.contains("systempreferences")
        {
            return true;
        }
    }

    let uninstall_mode = std::env::var("MOLE_UNINSTALL_MODE").unwrap_or_default() == "1";

    // 4. Container/Group Container bundle id 提取(对齐 SH 第 808-818 行)
    if let Some(bundle_id) = extract_bundle_from_path(path) {
        // Cache/tmp 目录是可重建的,设置 passthrough 标记
        if path.contains("/Data/Library/Caches/") || path.contains("/Data/tmp/") {
            container_cache_path = true;
        } else if !uninstall_mode && should_protect_data(&bundle_id) {
            return true;
        }
    }

    // 4b. 用户级 ~/Library/Caches/ 目录：缓存天然可重建，不应被 should_protect_data
    //     的 com.apple.* 前缀一刀切拦截（否则 com.apple.appstoreagent / iTunes /
    //     AppleMediaServicesUI 等数十个 Apple 服务缓存全部跳过，扫描量严重偏低）。
    //     真正关键的系统缓存已由上方 keyword 检查（Spotlight / IconServices /
    //     SystemPreferences 等）和下方硬编码保护（finder / dock 等）覆盖。
    //     对齐柠檬：柠檬仅排除 com.apple.Spotlight / IconServices / LaunchServices-*，
    //     其余 com.apple.* 缓存正常扫描清理。
    if !container_cache_path {
        let user_caches_marker = format!("{}/Library/Caches/", crate::core::base::home_dir());
        if path.contains(&user_caches_marker) {
            container_cache_path = true;
        }
    }

    // 5. 硬编码关键路径(对齐 SH 第 822-825 行)
    if wildcard_match(path, "*com.apple.Settings*")
        || wildcard_match(path, "*com.apple.SystemSettings*")
        || wildcard_match(path, "*com.apple.controlcenter*")
        || wildcard_match(path, "*com.apple.finder*")
        || wildcard_match(path, "*com.apple.dock*")
    {
        return true;
    }

    // 6. 关键偏好文件 + 用户数据(对齐 SH 第 828-848 行)
    if wildcard_match(path, "*/Library/Preferences/com.apple.dock.plist")
        || wildcard_match(path, "*/Library/Preferences/com.apple.finder.plist")
        || path.ends_with("/Library/Logs/mole")
        || path.contains("/Library/Logs/molan/")
        || wildcard_match(path, "*/ByHost/com.apple.bluetooth.*")
        || wildcard_match(path, "*/ByHost/com.apple.wifi.*")
        || wildcard_match(
            path,
            "*/Library/Preferences/com.apple.networkextension*.plist",
        )
        || path.contains("/Library/Mobile Documents")
        || path.contains("/Mobile Documents")
        || lower.contains("com.apple.coreaudio")
        || lower.contains("com.apple.audio.")
        || lower.contains("coreaudiod")
    {
        return true;
    }

    // 7. 全路径 vs 保护表匹配(只在非 container cache passthrough 场景下走)
    if !container_cache_path {
        if uninstall_mode {
            // uninstall 模式:先放行 Apple 可卸载,再阻拦 system-critical
            if APPLE_UNINSTALLABLE_APPS
                .iter()
                .any(|p| bundle_matches_pattern(path, p))
            {
                return false;
            }
            if SYSTEM_CRITICAL_BUNDLES
                .iter()
                .any(|p| bundle_matches_pattern(path, p))
            {
                return true;
            }
        } else {
            // 正常 cleanup 模式:system-critical + data-protected 全部保护
            if SYSTEM_CRITICAL_BUNDLES
                .iter()
                .chain(DATA_PROTECTED_BUNDLES.iter())
                .any(|p| bundle_matches_pattern(path, p))
            {
                return true;
            }
            // 文件名级别 fallback(对齐 SH 第 880-885 行)
            if let Some(filename) = Path::new(path).file_name().and_then(|s| s.to_str()) {
                if should_protect_data(filename) {
                    return true;
                }
            }
        }
    }

    false
}

pub fn is_path_whitelisted(path: &str, whitelist_patterns: &[String]) -> bool {
    if path.is_empty() || whitelist_patterns.is_empty() {
        return false;
    }
    let mut target = normalize_slashes(path.trim_end_matches('/'));
    if target.is_empty() {
        target = "/".to_string();
    }
    for p in whitelist_patterns {
        let mut check = normalize_slashes(p.trim_end_matches('/'));
        if check.is_empty() {
            check = "/".to_string();
        }
        let has_glob = check.contains('*') || check.contains('?') || check.contains('[');
        if target == check || wildcard_match(&target, &check) {
            return true;
        }
        if check.starts_with(&(target.clone() + "/")) {
            return true;
        }
        if !has_glob && target.starts_with(&(check + "/")) {
            return true;
        }
    }
    false
}

// ============================================================================
// 对齐 shell lib/core/app_protection.sh 新增辅助函数
// ============================================================================

/// 对齐 SH 第 966-977 行 `_mole_uninstall_is_common_app_name`
fn is_common_app_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "music"
            | "notes"
            | "photos"
            | "finder"
            | "safari"
            | "preview"
            | "calendar"
            | "contacts"
            | "messages"
            | "reminders"
            | "clock"
            | "weather"
            | "stocks"
            | "books"
            | "news"
            | "podcasts"
            | "voice"
            | "files"
            | "store"
            | "system"
            | "helper"
            | "agent"
            | "daemon"
            | "service"
            | "update"
            | "sync"
            | "backup"
            | "cloud"
            | "manager"
            | "monitor"
            | "server"
            | "client"
            | "worker"
            | "runner"
            | "launcher"
            | "driver"
            | "plugin"
            | "extension"
            | "widget"
            | "utility"
    )
}

/// 对齐 SH 第 981-993 行 `_mole_uninstall_vendor_product_tokens`
/// 从 bundle_id 提取 (vendor_token, product_token)
/// 例: "com.adobe.Photoshop" → ("Adobe", "Photoshop")
fn vendor_product_tokens(bundle_id: &str) -> Option<(String, String)> {
    if !is_reverse_dns_bundle(bundle_id) {
        return None;
    }
    let mut segments: Vec<&str> = bundle_id.split('.').collect();
    if segments.len() < 3 {
        return None;
    }
    let product_token = segments.pop()?;
    let vendor_token = segments.pop()?;

    // 两个 token 都必须 ≥3 字符，首字符字母数字
    fn valid_token(t: &str) -> bool {
        t.len() >= 3
            && t.starts_with(|c: char| c.is_ascii_alphanumeric())
            && t.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    }
    if !valid_token(vendor_token) || !valid_token(product_token) {
        return None;
    }
    Some((vendor_token.to_string(), product_token.to_string()))
}

/// 对齐 SH 第 995-1009 行 `_mole_uninstall_name_variant_matches`
/// candidate_lower 是否与 variant 列表中任一项精确匹配或以 variant+分隔符 开头
fn name_variant_matches(candidate_lower: &str, variants: &[String]) -> bool {
    for v in variants {
        if v.is_empty() {
            continue;
        }
        if candidate_lower == v.as_str() {
            return true;
        }
        if candidate_lower.starts_with(&format!("{v} ")) {
            return true;
        }
        if candidate_lower.starts_with(&format!("{v}-")) {
            return true;
        }
        if candidate_lower.starts_with(&format!("{v}_")) {
            return true;
        }
        if candidate_lower.starts_with(&format!("{v}.")) {
            return true;
        }
    }
    false
}

/// 对齐 SH base.sh `mole_name_starts_with_bundle_id_boundary`：
/// name == bundle_id 或 name == bundle_id.*（bundle_id 后跟 `.` 边界）。
fn name_starts_with_bundle_id_boundary(name: &str, bundle_id: &str) -> bool {
    if !is_reverse_dns_bundle(bundle_id) {
        return false;
    }
    name == bundle_id
        || name
            .strip_prefix(bundle_id)
            .is_some_and(|rest| rest.starts_with('.'))
}

/// 对齐 SH base.sh `mole_name_has_bundle_id_boundary`：
/// name == bundle_id / bundle_id.* / *.bundle_id / *.bundle_id.*
fn name_has_bundle_id_boundary(name: &str, bundle_id: &str) -> bool {
    if name_starts_with_bundle_id_boundary(name, bundle_id) {
        return true;
    }
    if !is_reverse_dns_bundle(bundle_id) {
        return false;
    }
    let dotted = format!(".{bundle_id}");
    name.ends_with(&dotted) || name.contains(&format!("{dotted}."))
}

/// 对齐 SH `_path_belongs_to_independent_cli`：判断 path 是否是同名独立 CLI 工具的 dotdir。
/// 卸载 GUI app 时跳过这些 dotdir，避免误删同名 CLI 工具的状态（Claude.app 不删 ~/.claude）。
fn path_belongs_to_independent_cli(path: &str, home: &str) -> bool {
    if path.is_empty() {
        return false;
    }
    let Some(slash) = path.rfind('/') else {
        return false;
    };
    let base = &path[slash + 1..];
    let parent = &path[..slash];
    let lc_name = base.trim_start_matches('.').to_ascii_lowercase();
    if lc_name.is_empty() {
        return false;
    }
    // deny-list：同名 CLI 工具（claude / opencode / codex / gemini）
    if !matches!(lc_name.as_str(), "claude" | "opencode" | "codex" | "gemini") {
        return false;
    }
    let config = format!("{home}/.config");
    let local_share = format!("{home}/.local/share");
    let cache = format!("{home}/.cache");
    parent == home
        || parent == config.as_str()
        || parent == local_share.as_str()
        || parent == cache.as_str()
}

/// 直接读 Info.plist 里的 CFBundleIdentifier（接受 plist 完整路径）。
/// 纯 Rust 解析（原 `plutil -extract ... raw` 子进程的原生替代）。
fn read_bundle_id_from_plist(plist_path: &str) -> String {
    super::bundle_id_anchor::read_bundle_id_from_plist(Path::new(plist_path))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s != "(null)")
        .unwrap_or_default()
}

/// 递归收集 `dir` 下 `*/Contents/Info.plist`（对齐 SH `find -maxdepth 12 -path */Contents/Info.plist`）。
fn collect_info_plists(dir: &str, remaining_depth: usize, out: &mut Vec<String>) {
    if remaining_depth == 0 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if !is_dir {
            let name = entry.file_name().to_string_lossy().to_string();
            if name == "Info.plist" {
                let parent_is_contents = path
                    .parent()
                    .and_then(|p| p.file_name())
                    .and_then(|s| s.to_str())
                    == Some("Contents");
                if parent_is_contents {
                    out.push(path.to_string_lossy().to_string());
                }
            }
        } else {
            collect_info_plists(&path.to_string_lossy(), remaining_depth - 1, out);
        }
    }
}

/// 对齐 SH `_mole_uninstall_embedded_bundle_ids`：
/// 扫描 app 包内 .xpc / .appex / LoginItems 下内嵌 bundle 的 bundle id。
fn embedded_bundle_ids(app_path: &str, primary_bundle_id: &str) -> Vec<String> {
    if app_path.is_empty() || !Path::new(&format!("{app_path}/Contents")).is_dir() {
        return Vec::new();
    }
    // 安全校验：绝对路径、无换行、无 /..（对齐 SH 第 860 行）。
    if !app_path.starts_with('/') || app_path.contains('\n') || app_path.contains("/..") {
        return Vec::new();
    }

    let contents = format!("{app_path}/Contents");
    let mut info_plists = Vec::new();
    collect_info_plists(&contents, 12, &mut info_plists);

    let mut ids: Vec<String> = Vec::new();
    let mut scanned = 0usize;
    for info in info_plists {
        scanned += 1;
        if scanned > 128 {
            break;
        }
        let Some(bundle_root) = info.strip_suffix("/Contents/Info.plist") else {
            continue;
        };
        if bundle_root == app_path {
            continue;
        }
        let bundle_name = match bundle_root.rfind('/') {
            Some(i) => &bundle_root[i + 1..],
            None => bundle_root,
        };
        let ext = match bundle_name.rfind('.') {
            Some(i) => &bundle_name[i + 1..],
            None => "",
        }
        .to_ascii_lowercase();

        // 只接受 .xpc / .appex / LoginItems 下的 .app（对齐 SH 第 883-892 行）。
        match ext.as_str() {
            "xpc" | "appex" => {}
            "app" => {
                if !bundle_root.starts_with(&format!("{app_path}/Contents/Library/LoginItems/")) {
                    continue;
                }
            }
            _ => continue,
        }

        let embedded_id = read_bundle_id_from_plist(&info);
        if !is_reverse_dns_bundle(&embedded_id) {
            continue;
        }
        if embedded_id == primary_bundle_id {
            continue;
        }
        // 共享框架服务不归每个 app 所有。
        if embedded_id.starts_with("org.sparkle-project.") {
            continue;
        }
        ids.push(embedded_id);
    }
    ids.sort();
    ids.dedup();
    ids
}

pub fn find_app_files(_bundle_id: &str, _app_name: &str, _app_path: &str) -> Vec<String> {
    let mut out = Vec::new();
    log::info!("[uninstall.find_app_files] bundle={_bundle_id} app={_app_name} path={_app_path}");
    let home = std::env::var("HOME").unwrap_or_default();
    if (_bundle_id.is_empty() || _bundle_id == "unknown") && (_app_name.len() < 2) {
        return out;
    }
    let nospace_name = _app_name.replace(' ', "");
    let underscore_name = _app_name.replace(' ', "_");
    let hyphen_name = _app_name.replace(' ', "-");
    let lowercase_name = _app_name.to_ascii_lowercase();
    let lowercase_nospace = nospace_name.to_ascii_lowercase();
    let lowercase_hyphen = hyphen_name.to_ascii_lowercase();
    let lowercase_underscore = underscore_name.to_ascii_lowercase();
    // bundle_id 必须过 reverse-DNS 校验才允许进入路径拼接 / find 模式（对齐 SH 第 951-957 行）。
    // 否则一个含 `/`、`..` 或 glob 字符的畸形 bundle id 会拼出危险路径或过度匹配。
    let bundle_id_valid = is_reverse_dns_bundle(_bundle_id);

    let mut candidates: Vec<String> = Vec::new();
    if bundle_id_valid {
        candidates.extend([
            format!("{home}/Library/Application Support/{}", _bundle_id),
            format!("{home}/Library/Caches/{}", _bundle_id),
            format!("{home}/Library/Logs/{}", _bundle_id),
            format!("{home}/Library/Preferences/{}.plist", _bundle_id),
            format!("{home}/Library/Containers/{}", _bundle_id),
            format!("{home}/Library/Group Containers/{}", _bundle_id),
            format!(
                "{home}/Library/Saved Application State/{}.savedState",
                _bundle_id
            ),
            format!("{home}/Library/WebKit/{}", _bundle_id),
            format!(
                "{home}/Library/WebKit/com.apple.WebKit.WebContent/{}",
                _bundle_id
            ),
            format!("{home}/Library/HTTPStorages/{}", _bundle_id),
            format!("{home}/Library/HTTPStorages/{}.binarycookies", _bundle_id),
            format!("{home}/Library/Cookies/{}.binarycookies", _bundle_id),
            format!("{home}/Library/Application Scripts/{}", _bundle_id),
            format!("{home}/Library/SyncedPreferences/{}.plist", _bundle_id),
            format!("{home}/Library/Input Methods/{}.app", _bundle_id),
            // 新增:NSURLSession 下载缓存(对齐 SH 第 1110 行)
            format!(
                "{home}/Library/Caches/com.apple.nsurlsessiond/Downloads/{}",
                _bundle_id
            ),
            format!("{home}/Library/Autosave Information/{}", _bundle_id),
        ]);
    }
    if _app_name.len() >= 2 {
        candidates.push(format!("{home}/Library/Application Support/{}", _app_name));
        candidates.push(format!("{home}/Library/Caches/{}", _app_name));
        candidates.push(format!("{home}/Library/Logs/{}", _app_name));
        candidates.push(format!("{home}/Library/Preferences/{}", _app_name));
        candidates.push(format!("{home}/Library/Preferences/{}.plist", _app_name));
        candidates.push(format!(
            "{home}/Library/Saved Application State/{}.savedState",
            _app_name
        ));
        candidates.push(format!("{home}/Library/Services/{}.workflow", _app_name));
        candidates.push(format!(
            "{home}/Library/QuickLook/{}.qlgenerator",
            _app_name
        ));
        candidates.push(format!(
            "{home}/Library/Internet Plug-Ins/{}.plugin",
            _app_name
        ));
        candidates.push(format!(
            "{home}/Library/Audio/Plug-Ins/Components/{}.component",
            _app_name
        ));
        candidates.push(format!(
            "{home}/Library/Audio/Plug-Ins/VST/{}.vst",
            _app_name
        ));
        candidates.push(format!(
            "{home}/Library/Audio/Plug-Ins/VST3/{}.vst3",
            _app_name
        ));
        candidates.push(format!(
            "{home}/Library/Audio/Plug-Ins/Digidesign/{}.dpm",
            _app_name
        ));
        candidates.push(format!(
            "{home}/Library/PreferencePanes/{}.prefPane",
            _app_name
        ));
        candidates.push(format!("{home}/Library/Input Methods/{}.app", _app_name));
        candidates.push(format!("{home}/Library/Screen Savers/{}.saver", _app_name));
        candidates.push(format!("{home}/Library/Frameworks/{}.framework", _app_name));
        candidates.push(format!(
            "{home}/Library/Contextual Menu Items/{}.plugin",
            _app_name
        ));
        candidates.push(format!("{home}/Library/Spotlight/{}.mdimporter", _app_name));
        candidates.push(format!(
            "{home}/Library/ColorPickers/{}.colorPicker",
            _app_name
        ));
        candidates.push(format!("{home}/Library/Workflows/{}.workflow", _app_name));
        // 新增:对齐 SH 第 1026-1028 行的三个 bundle 路径
        candidates.push(format!(
            "{home}/Library/Address Book Plug-Ins/{}.bundle",
            _app_name
        ));
        candidates.push(format!("{home}/Library/Accessibility/{}.bundle", _app_name));
        candidates.push(format!(
            "{home}/Library/Mail/Bundles/{}.mailbundle",
            _app_name
        ));
        // XDG 根下的全小写形式(APFS 大小写不敏感,与 SH 的写法等价)
        candidates.push(format!("{home}/.config/{lowercase_name}"));
        candidates.push(format!("{home}/.cache/{lowercase_name}"));
        candidates.push(format!("{home}/.local/share/{lowercase_name}"));
        candidates.push(format!("{home}/.{lowercase_name}"));
        candidates.push(format!("{home}/.{}rc", _app_name));
        // 空格命名变体(对齐 SH 第 1060-1087 行)
        if _app_name.contains(' ') && _app_name.len() > 3 {
            candidates.push(format!("{home}/Library/Application Support/{nospace_name}"));
            candidates.push(format!("{home}/Library/Caches/{nospace_name}"));
            candidates.push(format!("{home}/Library/Logs/{nospace_name}"));
            candidates.push(format!("{home}/Library/Preferences/{nospace_name}"));
            candidates.push(format!("{home}/Library/Preferences/{nospace_name}.plist"));
            candidates.push(format!(
                "{home}/Library/Saved Application State/{nospace_name}.savedState"
            ));
            candidates.push(format!(
                "{home}/Library/Application Support/{underscore_name}"
            ));
            candidates.push(format!("{home}/Library/Application Support/{hyphen_name}"));
            candidates.push(format!("{home}/Library/Preferences/{underscore_name}"));
            candidates.push(format!(
                "{home}/Library/Preferences/{underscore_name}.plist"
            ));
            candidates.push(format!("{home}/Library/Preferences/{hyphen_name}"));
            candidates.push(format!("{home}/Library/Preferences/{hyphen_name}.plist"));
            candidates.push(format!("{home}/.config/{lowercase_nospace}"));
            candidates.push(format!("{home}/.config/{lowercase_hyphen}"));
            candidates.push(format!("{home}/.config/{lowercase_underscore}"));
            candidates.push(format!("{home}/.cache/{lowercase_nospace}"));
            candidates.push(format!("{home}/.cache/{lowercase_hyphen}"));
            candidates.push(format!("{home}/.cache/{lowercase_underscore}"));
            candidates.push(format!("{home}/.local/share/{lowercase_nospace}"));
            candidates.push(format!("{home}/.local/share/{lowercase_hyphen}"));
            candidates.push(format!("{home}/.local/share/{lowercase_underscore}"));
        }
    }

    // version/channel base name variants(对齐 SH 第 1090-1103 行:"Zed Nightly" -> "zed")
    if let Some(base_name) = extract_base_name(_app_name).filter(|b| b.len() > 2) {
        let base_lower = base_name.to_ascii_lowercase();
        candidates.push(format!("{home}/Library/Application Support/{base_name}"));
        candidates.push(format!("{home}/Library/Caches/{base_name}"));
        candidates.push(format!("{home}/Library/Logs/{base_name}"));
        candidates.push(format!("{home}/Library/Preferences/{base_name}"));
        candidates.push(format!("{home}/Library/Preferences/{base_name}.plist"));
        candidates.push(format!(
            "{home}/Library/Saved Application State/{base_name}.savedState"
        ));
        candidates.push(format!("{home}/.config/{base_lower}"));
        candidates.push(format!("{home}/.cache/{base_lower}"));
        candidates.push(format!("{home}/.local/share/{base_lower}"));
        candidates.push(format!("{home}/.{base_lower}"));
    }

    // bundle id 最后一段(leaf)比 display name 更精确时推导数据目录名
    // (对齐 SH 第 1018-1058 行):tdesktop 分支 "AyuGram" + one.ayugram.AyuGramDesktop
    // -> "Application Support/AyuGram Desktop"。
    if bundle_id_valid {
        for variant in bundle_leaf_variants(_bundle_id, _app_name) {
            candidates.push(format!("{home}/Library/Application Support/{variant}"));
            candidates.push(format!("{home}/Library/Caches/{variant}"));
            candidates.push(format!("{home}/Library/Logs/{variant}"));
            candidates.push(format!("{home}/Library/Preferences/{variant}.plist"));
            candidates.push(format!(
                "{home}/Library/Saved Application State/{variant}.savedState"
            ));
        }
    }
    let mut matching: Vec<String> = candidates
        .into_par_iter()
        .filter(|c| {
            !c.contains("//")
                && Path::new(c).exists()
                && !path_belongs_to_independent_cli(c, home.as_str())
        })
        .collect();
    out.append(&mut matching);
    out.sort();
    out.dedup();
    // user launch agents by bundle id(对齐 SH 第 1105-1107 行 wildcard 扫描)
    if bundle_id_valid {
        let la = format!("{home}/Library/LaunchAgents");
        if let Ok(rd) = std::fs::read_dir(&la) {
            for e in rd.flatten() {
                let p = e.path();
                if let Some(name) = p.file_name().and_then(|s| s.to_str()) {
                    if name.ends_with(".plist")
                        && name_starts_with_bundle_id_boundary(name, _bundle_id)
                    {
                        out.push(p.to_string_lossy().to_string());
                    }
                }
            }
        }
    }

    // user launch agents by app name(对齐 SH 第 1158-1173 行)
    // 安全阀:小于 5 字符的 app_name 跳过(避免 "Time" 这类匹配到无关 plist)
    // 阻断 list:常见歧义词
    if _app_name.len() >= 5 {
        const COMMON_WORDS: &[&str] = &[
            "Music",
            "Notes",
            "Photos",
            "Finder",
            "Safari",
            "Preview",
            "Calendar",
            "Contacts",
            "Messages",
            "Reminders",
            "Clock",
            "Weather",
            "Stocks",
            "Books",
            "News",
            "Podcasts",
            "Voice",
            "Files",
            "Store",
            "System",
            "Helper",
            "Agent",
            "Daemon",
            "Service",
            "Update",
            "Sync",
            "Backup",
            "Cloud",
            "Manager",
            "Monitor",
            "Server",
            "Client",
            "Worker",
            "Runner",
            "Launcher",
            "Driver",
            "Plugin",
            "Extension",
            "Widget",
            "Utility",
        ];
        if !COMMON_WORDS.iter().any(|w| *w == _app_name) {
            let la = format!("{home}/Library/LaunchAgents");
            if let Ok(rd) = std::fs::read_dir(&la) {
                for e in rd.flatten() {
                    let p = e.path();
                    if let Some(name) = p.file_name().and_then(|s| s.to_str()) {
                        if name.starts_with("com.apple.") {
                            continue;
                        }
                        if !name.ends_with(".plist") {
                            continue;
                        }
                        if name.contains(_app_name) {
                            out.push(p.to_string_lossy().to_string());
                        }
                    }
                }
            }
        }
    }

    // 派生 bundle id 模糊匹配(对齐 SH 第 1120-1143 行):
    // 一些 share extension / FileProvider / app extension 用 "<bundle_id>.foo" 形式作目录名,
    // 这里按 bundle id 边界匹配(避免 com.foo 误匹配 com.foobar)。
    if bundle_id_valid {
        let derived_roots = [
            format!("{home}/Library/Application Scripts"),
            format!("{home}/Library/Containers"),
            format!("{home}/Library/Application Support/FileProvider"),
        ];
        for root in &derived_roots {
            if !Path::new(root).is_dir() {
                continue;
            }
            if let Ok(rd) = std::fs::read_dir(root) {
                for e in rd.flatten() {
                    let p = e.path();
                    if !p.is_dir() {
                        continue;
                    }
                    if let Some(name) = p.file_name().and_then(|s| s.to_str()) {
                        if name_has_bundle_id_boundary(name, _bundle_id) {
                            let path_str = p.to_string_lossy().to_string();
                            if !out.contains(&path_str) {
                                out.push(path_str);
                            }
                        }
                    }
                }
            }
        }
    }

    // Zed family special case: dev.zed.Zed-* http storage cross-channel leftovers
    if _bundle_id.starts_with("dev.zed.Zed-") {
        let zed_http = format!("{home}/Library/HTTPStorages");
        if let Ok(rd) = std::fs::read_dir(&zed_http) {
            for e in rd.flatten() {
                let p = e.path();
                if let Some(name) = p.file_name().and_then(|s| s.to_str()) {
                    if name.starts_with("dev.zed.Zed-") {
                        out.push(p.to_string_lossy().to_string());
                    }
                }
            }
        }
    }

    // ByHost preferences（对齐 SH 第 1190-1203 行 bundle id 边界匹配）
    let byhost = format!("{home}/Library/Preferences/ByHost");
    if bundle_id_valid && Path::new(&byhost).is_dir() {
        if let Ok(rd) = std::fs::read_dir(&byhost) {
            for e in rd.flatten() {
                let p = e.path();
                if let Some(name) = p.file_name().and_then(|s| s.to_str()) {
                    if name.ends_with(".plist")
                        && name_starts_with_bundle_id_boundary(name, _bundle_id)
                    {
                        out.push(p.to_string_lossy().to_string());
                    }
                }
            }
        }
    }

    // Group containers fuzzy match by bundle id（对齐 SH 第 1226-1240 行边界匹配）
    if bundle_id_valid {
        let gc = format!("{home}/Library/Group Containers");
        if let Ok(rd) = std::fs::read_dir(&gc) {
            for e in rd.flatten() {
                let p = e.path();
                if let Some(name) = p.file_name().and_then(|s| s.to_str()) {
                    if name_has_bundle_id_boundary(name, _bundle_id) {
                        out.push(p.to_string_lossy().to_string());
                    }
                }
            }
        }
    }

    // sharedfilelist sfl4
    if bundle_id_valid {
        let sfl_root = format!("{home}/Library/Application Support/com.apple.sharedfilelist");
        if let Ok(level1) = std::fs::read_dir(&sfl_root) {
            for d in level1.flatten() {
                let dp = d.path();
                if !dp.is_dir() {
                    continue;
                }
                if let Ok(files) = std::fs::read_dir(&dp) {
                    for f in files.flatten() {
                        let fp = f.path();
                        if let Some(n) = fp.file_name().and_then(|s| s.to_str()) {
                            if n == format!("{_bundle_id}.sfl4") {
                                out.push(fp.to_string_lossy().to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    // Helper 扩展 / XPC 服务可持久化自己的 bundle-id keyed 用户数据。
    // 只读选中 app 内有界的嵌入 bundle id，再映射到精确 ~/Library 路径（对齐 SH 第 1299-1340 行）。
    if bundle_id_valid && !_app_path.is_empty() {
        for embedded_id in embedded_bundle_ids(_app_path, _bundle_id) {
            for candidate in [
                format!("{home}/Library/Application Scripts/{embedded_id}"),
                format!("{home}/Library/Application Support/FileProvider/{embedded_id}"),
                format!("{home}/Library/Caches/{embedded_id}"),
                format!("{home}/Library/Containers/{embedded_id}"),
                format!("{home}/Library/HTTPStorages/{embedded_id}"),
                format!("{home}/Library/HTTPStorages/{embedded_id}.binarycookies"),
                format!("{home}/Library/Preferences/{embedded_id}.plist"),
                format!("{home}/Library/WebKit/{embedded_id}"),
            ] {
                if Path::new(&candidate).exists() {
                    out.push(candidate);
                }
            }
            // ByHost 下 embedded_id.*.plist
            if Path::new(&byhost).is_dir() {
                if let Ok(rd) = std::fs::read_dir(&byhost) {
                    for e in rd.flatten() {
                        let p = e.path();
                        if let Some(name) = p.file_name().and_then(|s| s.to_str()) {
                            if name.starts_with(&format!("{embedded_id}."))
                                && name.ends_with(".plist")
                            {
                                out.push(p.to_string_lossy().to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    // 工具链启发式。sibling guard 生效时整体关闭（对齐 SH `MOLE_UNINSTALL_SIBLING_SURVIVES=1`）：
    // 这些匹配靠 app_name/bundle_id 子串，同 bundle id 兄弟（Xcode.app vs Xcode-beta.app）会共享
    // DerivedData 等缓存，仅降级 bundle_id 挡不住，必须整体跳过。
    let collect_toolchain =
        std::env::var("MOLE_UNINSTALL_SIBLING_SURVIVES").unwrap_or_default() != "1";

    // Tool-specific leftovers (small high-signal subset)
    // VS Code 用户数据目录名既不匹配 app name("Visual Studio Code")也不匹配
    // bundle id("com.microsoft.VSCode"),按 SH 第 1473-1489 行显式覆盖稳定版 / Insiders。
    if is_vscode_bundle_id(_bundle_id) {
        for p in [
            format!("{home}/Library/Caches/com.microsoft.VSCode.ShipIt"),
            format!("{home}/Library/Caches/com.microsoft.VSCodeInsiders.ShipIt"),
        ] {
            if Path::new(&p).exists() {
                out.push(p);
            }
        }
        if _bundle_id.contains("Insiders") || _bundle_id.contains("insiders") {
            for p in [
                format!("{home}/.vscode-insiders"),
                format!("{home}/Library/Application Support/Code - Insiders"),
                format!("{home}/Library/Caches/com.microsoft.VSCodeInsiders"),
            ] {
                if Path::new(&p).exists() {
                    out.push(p);
                }
            }
        } else {
            for p in [
                format!("{home}/.vscode"),
                format!("{home}/Library/Application Support/Code"),
                format!("{home}/Library/Caches/com.microsoft.VSCode"),
            ] {
                if Path::new(&p).exists() {
                    out.push(p);
                }
            }
        }
    }
    if collect_toolchain && (_app_name.contains("Docker") || _bundle_id.contains("docker")) {
        // ~/.docker 根含 config.json（auth token）/ contexts（credentials）/ cli-plugins，
        // 只清可再生的 buildx / scan 子目录（对齐 SH 第 1490-1497 行）。
        for p in [
            format!("{home}/.docker/buildx"),
            format!("{home}/.docker/scan"),
        ] {
            if Path::new(&p).exists() {
                out.push(p);
            }
        }
    }
    if collect_toolchain
        && (_bundle_id == "com.maestro.studio" || lowercase_name.contains("maestro studio"))
    {
        let p = format!("{home}/.mobiledev");
        if Path::new(&p).exists() {
            out.push(p);
        }
    }
    // Anki 的 profile 目录(Anki2)存放牌组/媒体/备份,这里只收集启动器管理的
    // 支持文件(对齐 SH 第 1505-1510 行)。
    if collect_toolchain && (_bundle_id == "net.ankiweb.anki" || _app_name == "Anki") {
        let p = format!("{home}/Library/Application Support/AnkiProgramFiles");
        if Path::new(&p).exists() {
            out.push(p);
        }
    }
    if _bundle_id == "com.raycast.macos" {
        // Raycast v2 是独立 app(bundle id com.raycast-x.macos),每个 "*raycast*"
        // 扫描都要排除它的目录(对齐 SH 第 1512-1571 行)。
        let raycast_hit = |n: &str| {
            let low = n.to_ascii_lowercase();
            low.contains("raycast") && !low.contains("raycast-x")
        };
        // 标准用户目录 maxdepth 1(-type d)
        for dir in [
            format!("{home}/Library/Application Support"),
            format!("{home}/Library/Application Scripts"),
            format!("{home}/Library/Containers"),
        ] {
            if let Ok(rd) = std::fs::read_dir(&dir) {
                for e in rd.flatten() {
                    let p = e.path();
                    if !p.is_dir() {
                        continue;
                    }
                    if let Some(n) = p.file_name().and_then(|s| s.to_str()) {
                        if raycast_hit(n) {
                            out.push(p.to_string_lossy().to_string());
                        }
                    }
                }
            }
        }
        // 显式 Raycast 容器目录(硬编码残留)
        for p in [
            format!("{home}/Library/Containers/com.raycast.macos.BrowserExtension"),
            format!("{home}/Library/Containers/com.raycast.macos.RaycastAppIntents"),
        ] {
            if Path::new(&p).exists() {
                out.push(p);
            }
        }
        // Cache 更深搜索 maxdepth 2
        let caches = format!("{home}/Library/Caches");
        if let Ok(rd) = std::fs::read_dir(&caches) {
            for e in rd.flatten() {
                let p = e.path();
                if !p.is_dir() {
                    continue;
                }
                if let Some(n) = p.file_name().and_then(|s| s.to_str()) {
                    if raycast_hit(n) {
                        out.push(p.to_string_lossy().to_string());
                    }
                }
                // depth 2:一级子目录下的 *raycast* 目录
                if let Ok(rd2) = std::fs::read_dir(&p) {
                    for e2 in rd2.flatten() {
                        let p2 = e2.path();
                        if !p2.is_dir() {
                            continue;
                        }
                        if let Some(n2) = p2.file_name().and_then(|s| s.to_str()) {
                            if raycast_hit(n2) {
                                out.push(p2.to_string_lossy().to_string());
                            }
                        }
                    }
                }
            }
        }
        // VS Code 扩展存储
        let vscode_global = format!("{home}/Library/Application Support/Code/User/globalStorage");
        if let Ok(rd) = std::fs::read_dir(&vscode_global) {
            for e in rd.flatten() {
                let p = e.path();
                if !p.is_dir() {
                    continue;
                }
                if let Some(n) = p.file_name().and_then(|s| s.to_str()) {
                    if raycast_hit(n) {
                        out.push(p.to_string_lossy().to_string());
                    }
                }
            }
        }
    }

    // CrashReporter plists:名为 AppName_UUID.plist 的文件(非子目录),
    // 对齐 SH 第 1573-1588 行的通配符扫描。
    let crash_dir = format!("{home}/Library/Application Support/CrashReporter");
    if nospace_name.len() >= 3 && Path::new(&crash_dir).is_dir() {
        if let Ok(rd) = std::fs::read_dir(&crash_dir) {
            for e in rd.flatten() {
                let p = e.path();
                if !p.is_file() {
                    continue;
                }
                if let Some(name) = p.file_name().and_then(|s| s.to_str()) {
                    if name.ends_with(".plist")
                        && (name.starts_with(&format!("{_app_name}_"))
                            || name.starts_with(&format!("{nospace_name}_")))
                    {
                        out.push(p.to_string_lossy().to_string());
                    }
                }
            }
        }
    }

    // LaunchAgents by app name with common-word guard
    if _app_name.len() >= 5 {
        let common_words = [
            "Music",
            "Notes",
            "Photos",
            "Finder",
            "Safari",
            "Preview",
            "Calendar",
            "Contacts",
            "Messages",
            "Reminders",
            "Clock",
            "Weather",
            "Books",
            "News",
            "System",
            "Helper",
            "Agent",
            "Daemon",
            "Service",
            "Update",
            "Sync",
            "Backup",
            "Cloud",
            "Manager",
            "Monitor",
            "Server",
            "Client",
            "Worker",
            "Runner",
            "Launcher",
            "Driver",
            "Plugin",
            "Extension",
            "Widget",
            "Utility",
        ];
        if !common_words.iter().any(|w| *w == _app_name) {
            let la = format!("{home}/Library/LaunchAgents");
            if let Ok(rd) = std::fs::read_dir(&la) {
                for e in rd.flatten() {
                    let p = e.path();
                    if let Some(name) = p.file_name().and_then(|s| s.to_str()) {
                        if name.contains(_app_name)
                            && name.ends_with(".plist")
                            && !name.starts_with("com.apple.")
                        {
                            out.push(p.to_string_lossy().to_string());
                        }
                    }
                }
            }
        }
    }

    // Communication app residue
    if [
        "com.tencent.xinWeChat",
        "com.tencent.qq",
        "com.alibaba.DingTalkMac",
        "us.zoom.xos",
    ]
    .iter()
    .any(|x| _bundle_id == *x)
    {
        for p in [
            format!("{home}/Library/Containers/{}", _bundle_id),
            format!("{home}/Library/Group Containers/{}", _bundle_id),
            format!("{home}/Library/Application Support/{}", _bundle_id),
            format!("{home}/Library/Caches/{}", _bundle_id),
            format!("{home}/Library/Logs/{}", _bundle_id),
        ] {
            if Path::new(&p).exists() {
                out.push(p);
            }
        }
    }

    // Cloud sync residue
    if _bundle_id.to_ascii_lowercase().contains("dropbox")
        || _bundle_id.to_ascii_lowercase().contains("onedrive")
        || _bundle_id.to_ascii_lowercase().contains("googledrive")
        || _bundle_id.to_ascii_lowercase().contains("google")
            && _bundle_id.to_ascii_lowercase().contains("drive")
    {
        for p in [
            format!("{home}/Library/Application Support/{}", _bundle_id),
            format!("{home}/Library/Caches/{}", _bundle_id),
            format!("{home}/Library/Logs/{}", _bundle_id),
            format!("{home}/Library/Preferences/{}.plist", _bundle_id),
        ] {
            if Path::new(&p).exists() {
                out.push(p);
            }
        }
    }

    // VPN/proxy residue
    if _bundle_id.to_ascii_lowercase().contains("clash")
        || _bundle_id.to_ascii_lowercase().contains("surge")
        || _bundle_id.to_ascii_lowercase().contains("shadow")
        || _bundle_id.to_ascii_lowercase().contains("v2ray")
        || _bundle_id.to_ascii_lowercase().contains("openvpn")
        || _bundle_id.to_ascii_lowercase().contains("tailscale")
        || _bundle_id.to_ascii_lowercase().contains("zerotier")
    {
        for p in [
            format!("{home}/Library/Application Support/{}", _bundle_id),
            format!("{home}/Library/Caches/{}", _bundle_id),
            format!("{home}/Library/Logs/{}", _bundle_id),
            format!("{home}/Library/Preferences/{}.plist", _bundle_id),
            format!("{home}/.config/{}", _app_name.to_ascii_lowercase()),
        ] {
            if Path::new(&p).exists() {
                out.push(p);
            }
        }
    }

    // DevEco Studio（对齐 SH 第 1390-1401 行）
    // 只清可再生的 cache/log；跳过 ~/DevEcoStudioProjects（项目源码）、~/HarmonyOS、~/Huawei、
    // ~/DevEco-Studio、~/.huawei / ~/.ohos（账号 token / 签名 profile / SDK config）。
    if collect_toolchain
        && (_app_name.contains("DevEco")
            || _app_name.to_ascii_lowercase().contains("deveco")
            || (_bundle_id.to_ascii_lowercase().contains("huawei")
                && _bundle_id.to_ascii_lowercase().contains("deveco")))
    {
        for p in [
            format!("{home}/Library/Caches/Huawei"),
            format!("{home}/Library/Logs/Huawei"),
        ] {
            if Path::new(&p).exists() {
                out.push(p);
            }
        }
    }

    // Android Studio（对齐 SH 第 1403-1426 行）
    // 只清 ~/.android 下可再生的 cache；跳过 ~/AndroidStudioProjects（项目源码）、
    // ~/Library/Android（SDK，数 GB）、~/.android 根（debug.keystore 签名密钥 / adbkey / avd 镜像）。
    if collect_toolchain
        && (_app_name.to_ascii_lowercase().contains("android studio")
            || (_bundle_id.to_ascii_lowercase().contains("android")
                && _bundle_id.to_ascii_lowercase().contains("studio")))
    {
        for p in [
            format!("{home}/.android/cache"),
            format!("{home}/.android/build-cache"),
            format!("{home}/.android/breakpad"),
        ] {
            if Path::new(&p).exists() {
                out.push(p);
            }
        }
        let g = format!("{home}/Library/Application Support/Google");
        if let Ok(rd) = std::fs::read_dir(&g) {
            for e in rd.flatten() {
                let p = e.path();
                if let Some(n) = p.file_name().and_then(|s| s.to_str()) {
                    if n.starts_with("AndroidStudio") {
                        out.push(p.to_string_lossy().to_string());
                    }
                }
            }
        }
    }

    // Xcode（对齐 SH 第 1428-1445 行）
    // 只清可再生的 build/device caches；跳过 ~/Library/Developer 根（Toolchains、Archives、
    // UserData、CoreSimulator/Devices、provisioning profiles）。
    if collect_toolchain
        && (_app_name.to_ascii_lowercase().contains("xcode")
            || (_bundle_id.to_ascii_lowercase().contains("apple")
                && _bundle_id.to_ascii_lowercase().contains("xcode")))
    {
        for p in [
            format!("{home}/Library/Developer/Xcode/DerivedData"),
            format!("{home}/Library/Developer/Xcode/iOS DeviceSupport"),
            format!("{home}/Library/Developer/Xcode/macOS DeviceSupport"),
            format!("{home}/Library/Developer/Xcode/watchOS DeviceSupport"),
            format!("{home}/Library/Developer/Xcode/tvOS DeviceSupport"),
            format!("{home}/Library/Developer/Xcode/xrOS DeviceSupport"),
            format!("{home}/Library/Developer/CoreSimulator/Caches"),
            format!("{home}/.Xcode"),
        ] {
            if Path::new(&p).exists() {
                out.push(p);
            }
        }
    }

    // JetBrains families
    if collect_toolchain
        && (_bundle_id.to_ascii_lowercase().contains("jetbrains")
            || [
                "IntelliJ", "PyCharm", "WebStorm", "GoLand", "RubyMine", "PhpStorm", "CLion",
                "DataGrip", "Rider",
            ]
            .iter()
            .any(|k| _app_name.contains(k)))
    {
        for base in [
            format!("{home}/Library/Application Support/JetBrains"),
            format!("{home}/Library/Caches/JetBrains"),
            format!("{home}/Library/Logs/JetBrains"),
        ] {
            if let Ok(rd) = std::fs::read_dir(&base) {
                for e in rd.flatten() {
                    let p = e.path();
                    if let Some(n) = p.file_name().and_then(|s| s.to_str()) {
                        if n.starts_with(_app_name) {
                            out.push(p.to_string_lossy().to_string());
                        }
                    }
                }
            }
        }
    }

    // Engines
    if collect_toolchain && _app_name.to_ascii_lowercase().contains("unity") {
        let p = format!("{home}/Library/Unity");
        if Path::new(&p).exists() {
            out.push(p);
        }
    }
    if collect_toolchain && _app_name.to_ascii_lowercase().contains("unreal") {
        let p = format!("{home}/Library/Application Support/Epic");
        if Path::new(&p).exists() {
            out.push(p);
        }
    }
    if collect_toolchain && _app_name.to_ascii_lowercase().contains("godot") {
        let p = format!("{home}/Library/Application Support/Godot");
        if Path::new(&p).exists() {
            out.push(p);
        }
    }

    // 厂商嵌套扫描：~/Library/Application Support/Adobe/Photoshop 等
    out.extend(find_vendor_nested_app_paths(
        _bundle_id,
        _app_name,
        &[
            &format!("{home}/Library/Application Support"),
            &format!("{home}/Library/Caches"),
            &format!("{home}/Library/Logs"),
        ],
    ));

    out.sort();
    out.dedup();
    // PureMac 安全加固：过滤高风险 dotfile/dotdir（如 ~/.claude、~/.ssh）
    out = crate::core::high_risk_dotpaths::filter_high_risk_paths(out, &home);
    log::info!(
        "[uninstall.find_app_files] bundle={_bundle_id} app={_app_name} result_count={} paths={:?}",
        out.len(),
        out
    );
    out
}

pub fn get_diagnostic_report_paths_for_app(
    _app_path: &str,
    app_name: &str,
    base: &str,
) -> Vec<String> {
    let mut out = Vec::new();
    if _app_path.is_empty() || app_name.is_empty() || base.is_empty() {
        return out;
    }
    let dir = Path::new(base);
    if !dir.is_dir() {
        return out;
    }

    let nospace_name = app_name.replace(' ', "");
    let exec_name = read_bundle_executable(_app_path).unwrap_or_default();
    let prefix = if exec_name.is_empty() {
        nospace_name
    } else {
        exec_name
    };
    if prefix.len() < 3 {
        return out;
    }

    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let path = e.path();
            if !path.is_file() {
                continue;
            }
            let Some(base_name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let has_prefix = base_name.starts_with(&format!("{prefix}."))
                || base_name.starts_with(&format!("{prefix}_"))
                || base_name.starts_with(&format!("{prefix}-"));
            if !has_prefix {
                continue;
            }
            if base_name.ends_with(".ips")
                || base_name.ends_with(".crash")
                || base_name.ends_with(".spin")
                || base_name.ends_with(".diag")
            {
                out.push(path.to_string_lossy().to_string());
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// 对齐 SH 第 1014-1051 行 `find_vendor_nested_app_paths`
/// 扫描厂商嵌套目录，如 ~/Library/Application Support/Adobe/Photoshop
fn find_vendor_nested_app_paths(
    bundle_id: &str,
    app_name: &str,
    root_dirs: &[&str],
) -> Vec<String> {
    let mut out = Vec::new();
    if app_name.len() < 4 || is_common_app_name(app_name) {
        return out;
    }
    let (vendor_token, product_token) = match vendor_product_tokens(bundle_id) {
        Some(t) => t,
        None => return out,
    };
    let vendor_lower = vendor_token.to_ascii_lowercase();

    // 构建名称变体列表
    let app_lower = app_name.to_ascii_lowercase();
    let nospace_lower = app_name.replace(' ', "").to_ascii_lowercase();
    let hyphen_lower = app_name.replace(' ', "-").to_ascii_lowercase();
    let underscore_lower = app_name.replace(' ', "_").to_ascii_lowercase();
    let product_lower = product_token.to_ascii_lowercase();
    let variants = vec![
        app_lower,
        nospace_lower,
        hyphen_lower,
        underscore_lower,
        product_lower,
    ];

    for root in root_dirs {
        let root_p = Path::new(root);
        if !root_p.is_dir() {
            continue;
        }
        let Ok(level1_entries) = std::fs::read_dir(root) else {
            continue;
        };
        // mindepth 2 / maxdepth 2: 遍历 vendor 目录，再遍历产品目录
        for l1 in level1_entries.flatten() {
            let vendor_dir = l1.path();
            if !vendor_dir.is_dir() {
                continue;
            }
            let parent_base = vendor_dir
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            if parent_base.to_ascii_lowercase() != vendor_lower {
                continue;
            }
            let Ok(l2_entries) = std::fs::read_dir(&vendor_dir) else {
                continue;
            };
            for l2 in l2_entries.flatten() {
                let candidate = l2.path();
                if !candidate.is_dir() {
                    continue;
                }
                let child_base = candidate.file_name().and_then(|s| s.to_str()).unwrap_or("");
                let child_lower = child_base.to_ascii_lowercase();
                if name_variant_matches(&child_lower, &variants) {
                    out.push(candidate.to_string_lossy().to_string());
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// 对齐 SH 第 1054-1089 行 `find_shared_app_paths`
/// 扫描共享路径，如 /Users/Shared/Sibelius
fn find_shared_app_paths(bundle_id: &str, app_name: &str, root_dirs: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    if app_name.len() < 5 || is_common_app_name(app_name) {
        return out;
    }

    // 尝试解析 product_token（可选）
    let product_lower = vendor_product_tokens(bundle_id)
        .map(|(_, pt)| pt.to_ascii_lowercase())
        .unwrap_or_default();

    let app_lower = app_name.to_ascii_lowercase();
    let nospace_lower = app_name.replace(' ', "").to_ascii_lowercase();
    let hyphen_lower = app_name.replace(' ', "-").to_ascii_lowercase();
    let underscore_lower = app_name.replace(' ', "_").to_ascii_lowercase();
    let variants = vec![
        app_lower,
        nospace_lower,
        hyphen_lower,
        underscore_lower,
        product_lower,
    ];

    for root in root_dirs {
        let root_p = Path::new(root);
        if !root_p.is_dir() {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for e in entries.flatten() {
            let candidate = e.path();
            let base = candidate.file_name().and_then(|s| s.to_str()).unwrap_or("");
            let lower_base = base.to_ascii_lowercase();
            if name_variant_matches(&lower_base, &variants) {
                out.push(candidate.to_string_lossy().to_string());
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

pub fn find_app_system_files(_bundle_id: &str, _app_name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let nospace_name = _app_name.replace(' ', "");
    let underscore_name = _app_name.replace(' ', "_");
    let hyphen_name = _app_name.replace(' ', "-");
    let lowercase_hyphen = hyphen_name.to_ascii_lowercase();

    let mut patterns = vec![
        format!("/Library/Application Support/{}", _app_name),
        format!("/Library/Application Support/{}", _bundle_id),
        format!("/Library/LaunchAgents/{}.plist", _bundle_id),
        format!("/Library/LaunchDaemons/{}.plist", _bundle_id),
        format!("/Library/Preferences/{}.plist", _bundle_id),
        format!("/Library/Receipts/{}.bom", _bundle_id),
        format!("/Library/Receipts/{}.plist", _bundle_id),
        format!("/Library/Frameworks/{}.framework", _app_name),
        format!("/Library/Internet Plug-Ins/{}.plugin", _app_name),
        format!("/Library/Input Methods/{}.app", _app_name),
        format!("/Library/Input Methods/{}.app", _bundle_id),
        format!("/Library/Audio/Plug-Ins/Components/{}.component", _app_name),
        format!("/Library/Audio/Plug-Ins/VST/{}.vst", _app_name),
        format!("/Library/Audio/Plug-Ins/VST3/{}.vst3", _app_name),
        format!("/Library/Audio/Plug-Ins/Digidesign/{}.dpm", _app_name),
        format!("/Library/QuickLook/{}.qlgenerator", _app_name),
        format!("/Library/PreferencePanes/{}.prefPane", _app_name),
        format!("/Library/Screen Savers/{}.saver", _app_name),
        format!("/Library/Caches/{}", _bundle_id),
        format!("/Library/Caches/{}", _app_name),
        format!("/Library/Extensions/{}.kext", _app_name),
        format!("/Library/StartupItems/{}", _app_name),
        format!("/Library/Logs/{}", _app_name),
        format!("/Library/Logs/{}", _bundle_id),
    ];
    if _app_name.len() > 3 && _app_name.contains(' ') {
        patterns.extend([
            format!("/Library/Application Support/{nospace_name}"),
            format!("/Library/Caches/{nospace_name}"),
            format!("/Library/Logs/{nospace_name}"),
            format!("/Library/Application Support/{underscore_name}"),
            format!("/Library/Application Support/{hyphen_name}"),
            format!("/Library/Caches/{hyphen_name}"),
            format!("/Library/Caches/{lowercase_hyphen}"),
        ]);
    }

    for p in patterns {
        if p == "/Library/Application Support"
            || p == "/Library/Caches"
            || p == "/Library/Logs"
            || p.ends_with("/Library/Application Support/")
            || p.ends_with("/Library/Caches/")
            || p.ends_with("/Library/Logs/")
        {
            continue;
        }
        if Path::new(&p).exists() {
            out.push(p);
        }
    }

    if is_reverse_dns_bundle(_bundle_id) {
        for base in ["/Library/LaunchAgents", "/Library/LaunchDaemons"] {
            if let Ok(rd) = std::fs::read_dir(base) {
                for e in rd.flatten() {
                    let p = e.path();
                    let Some(name) = p.file_name().and_then(|s| s.to_str()) else {
                        continue;
                    };
                    if name == format!("{_bundle_id}.plist")
                        || (name.starts_with(&format!("{_bundle_id}.")) && name.ends_with(".plist"))
                    {
                        out.push(p.to_string_lossy().to_string());
                    }
                }
            }
        }
    }

    if _app_name.len() > 3 {
        for base in ["/Library/LaunchAgents", "/Library/LaunchDaemons"] {
            if let Ok(rd) = std::fs::read_dir(base) {
                for e in rd.flatten() {
                    let p = e.path();
                    let Some(name) = p.file_name().and_then(|s| s.to_str()) else {
                        continue;
                    };
                    if name.contains(_app_name) && name.ends_with(".plist") {
                        out.push(p.to_string_lossy().to_string());
                    }
                }
            }
        }
    }

    if !_bundle_id.is_empty() && _bundle_id != "unknown" && _bundle_id.len() > 3 {
        if let Ok(rd) = std::fs::read_dir("/Library/PrivilegedHelperTools") {
            for e in rd.flatten() {
                let p = e.path();
                if let Some(name) = p.file_name().and_then(|s| s.to_str()) {
                    if name.starts_with(_bundle_id) {
                        out.push(p.to_string_lossy().to_string());
                    }
                }
            }
        }
        if let Ok(rd) = std::fs::read_dir("/private/var/db/receipts") {
            for e in rd.flatten() {
                let p = e.path();
                if let Some(name) = p.file_name().and_then(|s| s.to_str()) {
                    if name.contains(_bundle_id) {
                        out.push(p.to_string_lossy().to_string());
                    }
                }
            }
        }
    }

    if _bundle_id == "com.raycast.macos" {
        if let Ok(rd) = std::fs::read_dir("/Library/Application Support") {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    if let Some(name) = p.file_name().and_then(|s| s.to_str()) {
                        if name.to_ascii_lowercase().contains("raycast") {
                            out.push(p.to_string_lossy().to_string());
                        }
                    }
                }
            }
        }
    }

    // 厂商嵌套扫描：/Library/Application Support/Adobe/Photoshop 等
    out.extend(find_vendor_nested_app_paths(
        _bundle_id,
        _app_name,
        &[
            "/Library/Application Support",
            "/Library/Caches",
            "/Library/Logs",
        ],
    ));

    // /Users/Shared 共享路径扫描
    out.extend(find_shared_app_paths(
        _bundle_id,
        _app_name,
        &["/Users/Shared"],
    ));

    out.extend(find_app_receipt_files(_bundle_id, _app_name));
    out.sort();
    out.dedup();
    out
}

pub fn find_app_receipt_files(_bundle_id: &str, _app_name: &str) -> Vec<String> {
    if _bundle_id.is_empty() || _bundle_id == "unknown" {
        return Vec::new();
    }
    if !_bundle_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
    {
        return Vec::new();
    }

    let mut bom_files = Vec::new();
    if let Ok(rd) = std::fs::read_dir("/private/var/db/receipts") {
        for e in rd.flatten() {
            let p = e.path();
            if let Some(name) = p.file_name().and_then(|s| s.to_str()) {
                if name.starts_with(_bundle_id) && name.ends_with(".bom") {
                    bom_files.push(p.to_string_lossy().to_string());
                }
            }
        }
    }

    let mut receipt_files = Vec::new();
    for bom in bom_files {
        let out = Command::new("lsbom").args(["-f", "-s", &bom]).output();
        let Ok(out) = out else {
            continue;
        };
        if !out.status.success() {
            continue;
        }
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let mut clean = line.trim().trim_start_matches('.').to_string();
            if clean.is_empty() {
                continue;
            }
            if !clean.starts_with('/') {
                clean = format!("/{clean}");
            }
            if clean.contains("..") {
                continue;
            }
            clean = normalize_slashes(&clean);
            let safe = clean.starts_with("/Applications/")
                || clean.starts_with("/Library/Application Support/")
                || clean.starts_with("/Library/Caches/")
                || clean.starts_with("/Library/Logs/")
                || clean.starts_with("/Library/Preferences/")
                || clean.starts_with("/Library/LaunchAgents/")
                || clean.starts_with("/Library/LaunchDaemons/")
                || clean.starts_with("/Library/PrivilegedHelperTools/");
            let hard_block = clean.starts_with("/System/")
                || clean.starts_with("/usr/bin/")
                || clean.starts_with("/usr/lib/")
                || clean.starts_with("/bin/")
                || clean.starts_with("/sbin/")
                || clean.starts_with("/private/");
            if safe && !hard_block && Path::new(&clean).exists() && !should_protect_path(&clean) {
                if clean != "/Applications" && clean != "/Library" {
                    receipt_files.push(clean);
                }
            }
        }
    }
    receipt_files.sort();
    receipt_files.dedup();
    receipt_files
}

/// 读 CFBundleIdentifier（对齐 SH force_kill_app 第 2111 行的 plutil -extract）。
fn read_bundle_identifier(app_path: &str) -> Option<String> {
    super::bundle_id_anchor::read_bundle_id_of_app(Path::new(app_path))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s != "(null)")
}

pub fn force_kill_app(app_name: &str, app_path: &str) -> bool {
    if std::env::var("MOLE_DRY_RUN").unwrap_or_default() == "1" {
        return true;
    }

    let exec_name = read_bundle_executable(app_path).unwrap_or_default();
    let bundle_id = read_bundle_identifier(app_path).unwrap_or_default();
    let match_pattern = if exec_name.is_empty() {
        app_name.to_string()
    } else {
        exec_name
    };
    if match_pattern.is_empty() {
        return true;
    }

    // 系统进程名守卫（对齐 SH 第 2124-2129 行）：match_pattern 来自 CFBundleExecutable
    // （第三方 .app 可任意设置），拒绝精确匹配系统关键进程名，防止被伪装的 app 拿去杀 Finder/Dock。
    const SYSTEM_PROCESS_NAMES: &[&str] = &[
        "Finder",
        "Dock",
        "loginwindow",
        "WindowServer",
        "SystemUIServer",
        "launchd",
        "coreaudiod",
        "NotificationCenter",
        "ControlCenter",
        "Spotlight",
    ];
    if SYSTEM_PROCESS_NAMES.contains(&match_pattern.as_str()) {
        crate::core::log::debug_log(&format!(
            "force_kill_app: refusing to operate on system process name '{match_pattern}'"
        ));
        return false;
    }

    let is_running = || pgrep_x(&match_pattern);

    if !is_running() {
        return true;
    }

    // 优雅退出：先发 quit Apple Event，让 Tauri/Electron/SwiftUI app 走正常 terminate 流程。
    // 后台发起 + 轮询，权限弹窗 / 挂起的 app 不能卡死卸载（对齐 SH 第 2145-2173 行）。
    let test_mode = std::env::var("MOLE_TEST_MODE").unwrap_or_default() == "1"
        || std::env::var("MOLE_TEST_NO_AUTH").unwrap_or_default() == "1";
    if !test_mode {
        let quit_target = if is_reverse_dns_bundle(&bundle_id) {
            format!("id \"{bundle_id}\"")
        } else {
            let escaped = app_name.replace('\\', "\\\\").replace('"', "\\\"");
            format!("\"{escaped}\"")
        };
        let script = format!("tell application {quit_target} to quit");
        if let Ok(mut child) = Command::new("osascript").arg("-e").arg(&script).spawn() {
            for _ in 0..20 {
                if !is_running() {
                    let _ = child.kill();
                    let _ = child.wait();
                    return true;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    if !is_running() {
        return true;
    }

    // 阶梯：SIGTERM → SIGKILL → 缓存 sudo 下的 SIGKILL 重试。
    let _ = Command::new("pkill").args(["-x", &match_pattern]).output();
    std::thread::sleep(std::time::Duration::from_secs(2));
    if !is_running() {
        return true;
    }

    let _ = Command::new("pkill")
        .args(["-9", "-x", &match_pattern])
        .output();
    std::thread::sleep(std::time::Duration::from_secs(2));
    if !is_running() {
        return true;
    }

    let has_sudo = crate::core::sudo::sudo_output(&["/usr/bin/true"])
        .status
        .success();
    if has_sudo {
        let _ = crate::core::sudo::sudo_output(&["/usr/bin/pkill", "-9", "-x", &match_pattern]);
        std::thread::sleep(std::time::Duration::from_secs(2));
    }

    for _ in 0..3 {
        if !is_running() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    !is_running()
}

/// SH 第 1018-1058 行 bundle_leaf 推导的纯逻辑:返回通过取证条件的目录名变体。
/// leaf 单独不足取证(com.wrapper.GoogleChrome 会合成他人目录),必须 leaf 扩展
/// display name 本身:leaf >= 8 字符、含驼峰转折、小写 leaf 以小写无空格名开头
/// 且更长、余下部分以大写或数字开头。
fn bundle_leaf_variants(bundle_id: &str, app_name: &str) -> Vec<String> {
    let mut variants = Vec::new();
    let nospace = app_name.replace(' ', "");
    if app_name.len() < 3 || nospace.len() < 3 {
        return variants;
    }
    let leaf = bundle_id.rsplit('.').next().unwrap_or_default().to_string();
    let leaf_lower = leaf.to_ascii_lowercase();
    let nospace_lower = nospace.to_ascii_lowercase();
    let has_camel = leaf
        .as_bytes()
        .windows(2)
        .any(|w| w[0].is_ascii_lowercase() && w[1].is_ascii_uppercase());
    if leaf.len() >= 8
        && leaf != app_name
        && has_camel
        && leaf_lower.starts_with(&nospace_lower)
        && leaf_lower.len() > nospace_lower.len()
    {
        // SH 按无空格名长度切片(原始大小写),余下部分必须以大写或数字开头
        let rest = &leaf[nospace.len()..];
        if rest
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        {
            let rest_spaced = spread_camel_spaces(rest);
            for variant in [leaf, format!("{app_name} {rest_spaced}")] {
                if variant != app_name {
                    variants.push(variant);
                }
            }
        }
    }
    variants
}

/// SH 第 1477 行 `[[ "$bundle_id" =~ microsoft.*[vV][sS][cC]ode ]]` 的 Rust 等价:
/// "microsoft" 之后任意位置出现 "vscode"(V/S/C 大小写混合,"ode" 小写)。
fn is_vscode_bundle_id(bundle_id: &str) -> bool {
    let bytes = bundle_id.as_bytes();
    let Some(mpos) = bundle_id.find("microsoft") else {
        return false;
    };
    for i in (mpos + "microsoft".len())..bytes.len() {
        if i + 6 > bytes.len() {
            break;
        }
        let w = &bytes[i..i + 6];
        if matches!(w[0], b'v' | b'V')
            && matches!(w[1], b's' | b'S')
            && matches!(w[2], b'c' | b'C')
            && w[3] == b'o'
            && w[4] == b'd'
            && w[5] == b'e'
        {
            return true;
        }
    }
    false
}

/// SH app_protection.sh bundle_leaf 分支两条 sed 替换的 Rust 等价:
///   sed -E 's/([A-Z]+)([A-Z][a-z])/\1 \2/g; s/([a-z0-9])([A-Z])/\1 \2/g'
/// 即在大写连串后接"大写+小写"处、以及小写/数字后接大写处插入空格:
/// "XMLParser" -> "XML Parser","AyuGram" -> "Ayu Gram","Desktop" 不变。
/// 已与 macOS BSD sed 逐例对拍验证。
fn spread_camel_spaces(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(n + 4);
    let is_up = |c: char| c.is_ascii_uppercase();
    let mut i = 0;
    while i < n {
        // [A-Z]+ 后接 [A-Z][a-z]:在 run 末尾与最后那个大写之间插空格
        if is_up(chars[i]) && i + 2 < n && is_up(chars[i + 1]) && chars[i + 2].is_ascii_lowercase()
        {
            out.push(chars[i]);
            out.push(' ');
            i += 1;
            continue;
        }
        // [a-z0-9] 后接 [A-Z]
        if (chars[i].is_ascii_lowercase() || chars[i].is_ascii_digit())
            && i + 1 < n
            && is_up(chars[i + 1])
        {
            out.push(chars[i]);
            out.push(' ');
            i += 1;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn extract_base_name(app_name: &str) -> Option<String> {
    let suffixes = [
        "Nightly",
        "Beta",
        "Alpha",
        "Dev",
        "Canary",
        "Preview",
        "Insider",
        "Edge",
        "Stable",
        "Release",
        "RC",
        "LTS",
        "Developer Edition",
        "Technology Preview",
    ];
    for s in suffixes {
        let marker = format!(" {s}");
        if let Some(base) = app_name.strip_suffix(&marker) {
            if !base.trim().is_empty() {
                return Some(base.trim().to_string());
            }
        }
    }
    None
}

fn read_bundle_executable(app_path: &str) -> Option<String> {
    let plist_path = format!("{app_path}/Contents/Info.plist");
    if !Path::new(&plist_path).is_file() {
        return None;
    }
    let out = Command::new("defaults")
        .args(["read", &plist_path, "CFBundleExecutable"])
        .output()
        .ok()?;
    if out.status.success() {
        let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !v.is_empty() {
            return Some(v);
        }
    }
    None
}

fn is_reverse_dns_bundle(bundle_id: &str) -> bool {
    if bundle_id.is_empty() || bundle_id == "unknown" {
        return false;
    }
    let parts: Vec<&str> = bundle_id.split('.').collect();
    if parts.len() < 2 {
        return false;
    }
    for p in parts {
        if p.is_empty() {
            return false;
        }
        let mut chars = p.chars();
        let Some(first) = chars.next() else {
            return false;
        };
        if !first.is_ascii_alphanumeric() {
            return false;
        }
        if !chars.all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spread_camel_spaces_matches_bsd_sed() {
        // 与 macOS BSD sed 逐例对拍:
        // sed -E 's/([A-Z]+)([A-Z][a-z])/\1 \2/g; s/([a-z0-9])([A-Z])/\1 \2/g'
        assert_eq!(spread_camel_spaces("Desktop"), "Desktop");
        assert_eq!(spread_camel_spaces("XMLParser"), "XML Parser");
        assert_eq!(spread_camel_spaces("AyuGram"), "Ayu Gram");
        assert_eq!(spread_camel_spaces("TelegramDesktop"), "Telegram Desktop");
        assert_eq!(spread_camel_spaces("ABcd"), "A Bcd");
        assert_eq!(spread_camel_spaces("ABCd"), "AB Cd");
        assert_eq!(spread_camel_spaces("TTtop"), "T Ttop");
        assert_eq!(spread_camel_spaces("Dkt"), "Dkt");
        assert_eq!(spread_camel_spaces("Deskto"), "Deskto");
    }

    #[test]
    fn bundle_leaf_variants_match_sh_examples() {
        // SH 注释里的正例:tdesktop 分支
        assert_eq!(
            bundle_leaf_variants("one.ayugram.AyuGramDesktop", "AyuGram"),
            vec!["AyuGramDesktop".to_string(), "AyuGram Desktop".to_string()]
        );
        // 反例:包装器 app 的 leaf 不扩展自身 display name
        assert!(bundle_leaf_variants("com.wrapper.GoogleChrome", "Wrapper").is_empty());
        // 反例:分支名 64Gram 不匹配 leaf 前缀 org.fork.TelegramDesktop
        assert!(bundle_leaf_variants("org.fork.TelegramDesktop", "64Gram").is_empty());
        // 反例:leaf 太短(< 8)
        assert!(bundle_leaf_variants("com.dbx.app", "DBX").is_empty());
        // 反例:leaf 与 display name 相同
        assert!(bundle_leaf_variants("com.foo.AyuGram", "AyuGram").is_empty());
        // 反例:无驼峰转折(全小写)
        assert!(bundle_leaf_variants("com.foo.ayugramdesktop", "AyuGram").is_empty());
        // 反例:余下部分以小写字母开头(非大写/数字)
        assert!(bundle_leaf_variants("com.foo.AyuGramdesktop", "AyuGram").is_empty());
    }

    #[test]
    fn vscode_bundle_id_match_follows_sh_regex() {
        // SH: [[ "$bundle_id" =~ microsoft.*[vV][sS][cC]ode ]]
        assert!(is_vscode_bundle_id("com.microsoft.VSCode"));
        assert!(is_vscode_bundle_id("com.microsoft.VSCodeInsiders"));
        assert!(is_vscode_bundle_id("x.microsoft.y.Vscode.z"));
        assert!(!is_vscode_bundle_id("com.example.VSCode")); // 无 microsoft 前缀
        assert!(!is_vscode_bundle_id("com.microsoft.Foo"));
        assert!(!is_vscode_bundle_id(""));
    }
}
