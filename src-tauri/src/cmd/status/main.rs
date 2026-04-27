use std::io::IsTerminal;
use std::process::Command;

use clap::Parser;
use molestudio_lib::cmd::status::metrics::Collector;
use molestudio_lib::cmd::status::process_watch::ProcessWatchOptions;

#[derive(Parser)]
#[command(name = "rmole-status")]
struct Cli {
    #[arg(long, default_value_t = false)]
    json: bool,

    #[arg(long, default_value_t = 100.0)]
    proc_cpu_threshold: f64,

    #[arg(long, default_value_t = 300.0)]
    proc_cpu_window: f64,

    #[arg(long, default_value_t = true)]
    proc_cpu_alerts: bool,
}

fn main() {
    let cli = parse_cli_or_exit();

    let options = ProcessWatchOptions {
        enabled: cli.proc_cpu_alerts,
        cpu_threshold: cli.proc_cpu_threshold,
        window_secs: cli.proc_cpu_window,
    };

    let should_json = cli.json || !std::io::stdout().is_terminal();

    if should_json {
        run_json_mode(options);
    } else {
        run_tui_stub();
    }
}

fn parse_cli_or_exit() -> Cli {
    match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            let rendered = err.to_string();
            if let Some(flag) = extract_unknown_flag(&rendered) {
                eprintln!("flag provided but not defined: {}", normalize_go_style_flag(&flag));
                let usage_target = std::env::current_exe()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| "rmole-status".to_string());
                eprintln!("Usage of {usage_target}:");
                eprintln!("  -json");
                eprintln!("    \toutput metrics as JSON instead of TUI");
                eprintln!("  -proc-cpu-alerts");
                eprintln!("    \tenable persistent high-CPU process alerts (default true)");
                eprintln!("  -proc-cpu-threshold float");
                eprintln!("    \talert when a process stays above this CPU percent (default 100)");
                eprintln!("  -proc-cpu-window duration");
                eprintln!("    \tcontinuous duration a process must exceed the CPU threshold (default 5m0s)");
                std::process::exit(2);
            }

            eprint!("{rendered}");
            std::process::exit(2);
        }
    }
}

fn extract_unknown_flag(rendered: &str) -> Option<String> {
    let marker = "unexpected argument '";
    let start = rendered.find(marker)?;
    let after = &rendered[(start + marker.len())..];
    let end = after.find('\'')?;
    Some(after[..end].to_string())
}

fn normalize_go_style_flag(flag: &str) -> String {
    if let Some(rest) = flag.strip_prefix("--") {
        return format!("-{rest}");
    }
    flag.to_string()
}

fn run_json_mode(options: ProcessWatchOptions) {
    // Keep parity with Mole's status-go: if ps cannot be executed,
    // report collection failure and exit non-zero.
    if let Err(e) = Command::new("/bin/ps").arg("-A").output() {
        eprintln!("error collecting metrics: fork/exec /bin/ps: {}", e);
        std::process::exit(1);
    }

    let mut collector = Collector::new(options);

    let _first = collector.collect_first();

    let data = collector.collect_second();

    match serde_json::to_string_pretty(&data) {
        Ok(s) => println!("{}", s),
        Err(e) => {
            eprintln!("error encoding JSON: {}", e);
            std::process::exit(1);
        }
    }
}

fn run_tui_stub() {
    eprintln!("rmole-status: TUI mode not yet implemented (V1). Use --json for JSON output.");
    std::process::exit(1);
}
