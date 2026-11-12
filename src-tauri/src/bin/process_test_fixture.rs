use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::Duration;

fn main() {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    // The production launch path spawns games without arguments, so a fixture copy may
    // be configured by a sibling `fixture.json` instead of argv. This keeps each test
    // isolated to its own copied executable (no process-wide environment mutation).
    let (mode, args) = if args.is_empty() {
        match read_sibling_config() {
            Some((mode, args)) => (mode, args),
            None => (String::new(), Vec::new()),
        }
    } else {
        (args[0].to_string_lossy().into_owned(), args[1..].to_vec())
    };
    let exit = match mode.as_str() {
        "direct" => run_direct(args),
        "launcher" => run_launcher(args),
        _ => 64,
    };
    std::process::exit(exit);
}

fn read_sibling_config() -> Option<(String, Vec<std::ffi::OsString>)> {
    let directory = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let bytes = fs::read(directory.join("fixture.json")).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let mode = value.get("mode")?.as_str()?.to_string();
    let relative =
        |key: &str| -> Option<std::ffi::OsString> { Some(directory.join(value.get(key)?.as_str()?).into_os_string()) };
    let delay = value.get("delayMs")?.as_u64()?.to_string();
    let args = match mode.as_str() {
        "direct" => vec![
            relative("report")?,
            value.get("exit")?.as_i64()?.to_string().into(),
            delay.into(),
        ],
        "launcher" => {
            let mut args = vec![relative("parentReport")?, relative("childReport")?, delay.into()];
            // `childExe` is optional. When present, the launcher starts that
            // *different* binary, so the game child has its own executable path.
            // That is the case the explicit-child tracking has to cover.
            if let Some(child) = value.get("childExe").and_then(|value| value.as_str()) {
                args.push(directory.join(child).into_os_string());
            }
            args
        }
        _ => return None,
    };
    Some((mode, args))
}

fn run_direct(args: Vec<std::ffi::OsString>) -> i32 {
    let Some(report_path) = args.first() else { return 64 };
    let exit_code = args
        .get(1)
        .and_then(|value| value.to_string_lossy().parse::<i32>().ok())
        .unwrap_or(64);
    let delay_ms = args
        .get(2)
        .and_then(|value| value.to_string_lossy().parse::<u64>().ok())
        .unwrap_or(0);
    let reported: Vec<_> = args
        .iter()
        .skip(3)
        .map(|value| value.to_string_lossy().into_owned())
        .collect();
    let payload = serde_json::json!({
        "pid": std::process::id(),
        "cwd": std::env::current_dir().unwrap(),
        "args": reported,
    });
    if fs::write(report_path, payload.to_string()).is_err() {
        return 65;
    }
    thread::sleep(Duration::from_millis(delay_ms));
    exit_code
}

fn run_launcher(args: Vec<std::ffi::OsString>) -> i32 {
    let Some(parent_report) = args.first() else { return 64 };
    let Some(child_report) = args.get(1) else { return 64 };
    let delay_ms = args
        .get(2)
        .and_then(|value| value.to_string_lossy().parse::<u64>().ok())
        .unwrap_or(0);
    // The launcher starts a game child. Without an explicit child executable it
    // re-runs its own binary; with one, the child has a *different* executable
    // path, which is what the explicit-child tracking must handle.
    let child_executable = args
        .get(3)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_exe().unwrap());
    let child = match Command::new(child_executable)
        .args([
            "direct",
            child_report.to_string_lossy().as_ref(),
            "0",
            &delay_ms.to_string(),
        ])
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return 66,
    };
    let payload = serde_json::json!({ "launcher_pid": std::process::id(), "child_pid": child.id() });
    if fs::write(parent_report, payload.to_string()).is_err() {
        return 67;
    }
    0
}
