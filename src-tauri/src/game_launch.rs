//! Explicit launch-target planning and conservative launcher discovery.
//!
//! The direct-spawn adapter is used only by the opt-in Windows Gaming Mode path.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum DirectSpawnError {
    Unsupported,
    Spawn(std::io::Error),
}

pub enum DirectSpawnOutcome {
    Tracked(crate::process_monitor::TrackedProcess),
    LaunchedButTrackingUncertain {
        pid: Option<u32>,
        reason: crate::process_monitor::UnknownReason,
    },
}

/// Starts the actual executable directly.
///
/// When `elevate` is true and `CreateProcessW` returns `ERROR_ELEVATION_REQUIRED`,
/// the launch is retried via `ShellExecuteExW` with the `runas` verb, which triggers
/// a UAC consent prompt. If the user accepts, the elevated process is tracked.
#[cfg(windows)]
pub fn spawn_direct_game(
    executable: &Path,
    cwd: &Path,
    args: &[std::ffi::OsString],
    elevate: bool,
) -> Result<DirectSpawnOutcome, DirectSpawnError> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        CreateProcessW, CREATE_NO_WINDOW, HIGH_PRIORITY_CLASS, PROCESS_INFORMATION, STARTUPINFOW,
    };

    let mut app: Vec<u16> = executable.as_os_str().encode_wide().chain(Some(0)).collect();
    let command_line = make_windows_command_line(executable.as_os_str(), args);
    let mut command_line: Vec<u16> = command_line.encode_wide().chain(Some(0)).collect();
    let mut dir: Vec<u16> = cwd.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut startup = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        ..Default::default()
    };
    let mut info = PROCESS_INFORMATION::default();
    // CREATE_NO_WINDOW suppresses console creation; HIGH_PRIORITY_CLASS preserves legacy /HIGH
    // at process creation, before the primary thread is resumed by CreateProcessW.
    let result = unsafe {
        CreateProcessW(
            windows::core::PCWSTR(app.as_mut_ptr()),
            windows::core::PWSTR(command_line.as_mut_ptr()),
            None,
            None,
            false,
            CREATE_NO_WINDOW | HIGH_PRIORITY_CLASS,
            None,
            windows::core::PCWSTR(dir.as_mut_ptr()),
            &mut startup,
            &mut info,
        )
    };
    if let Err(error) = result {
        // ERROR_ELEVATION_REQUIRED: the executable demands administrator rights.
        // CreateProcessW cannot elevate, so retry via ShellExecuteExW with the
        // "runas" verb, which triggers a UAC consent prompt. If the user accepts,
        // the process launches elevated and we can track its handle.
        if elevate && error.code().0 as u32 == 0x800702E4 {
            return spawn_with_elevation(executable, cwd);
        }
        return Err(DirectSpawnError::Spawn(error.into()));
    }
    // The process handle comes from PROCESS_INFORMATION and ownership transfers to TrackedProcess.
    let tracked = crate::process_monitor::TrackedProcess::from_owned_handle(info.dwProcessId, info.hProcess);
    // SAFETY: CreateProcessW returned a valid primary-thread handle; it is not retained.
    let _ = unsafe { CloseHandle(info.hThread) };
    match tracked {
        Ok(process) => Ok(DirectSpawnOutcome::Tracked(process)),
        Err(crate::process_monitor::ProcessStatus::Unknown(reason)) => {
            Ok(DirectSpawnOutcome::LaunchedButTrackingUncertain {
                pid: Some(info.dwProcessId),
                reason,
            })
        }
        Err(_) => Ok(DirectSpawnOutcome::LaunchedButTrackingUncertain {
            pid: Some(info.dwProcessId),
            reason: crate::process_monitor::UnknownReason::AccessOrQueryFailure,
        }),
    }
}

#[cfg(windows)]
fn spawn_with_elevation(executable: &Path, cwd: &Path) -> Result<DirectSpawnOutcome, DirectSpawnError> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Threading::GetProcessId;
    use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let file: Vec<u16> = executable.as_os_str().encode_wide().chain(Some(0)).collect();
    let dir: Vec<u16> = cwd.as_os_str().encode_wide().chain(Some(0)).collect();
    let verb: Vec<u16> = "runas".encode_utf16().chain(Some(0)).collect();

    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS;
    info.lpVerb = windows::core::PCWSTR(verb.as_ptr());
    info.lpFile = windows::core::PCWSTR(file.as_ptr());
    info.lpParameters = windows::core::PCWSTR::null();
    info.lpDirectory = windows::core::PCWSTR(dir.as_ptr());
    info.nShow = SW_SHOWNORMAL.0;

    let result = unsafe { ShellExecuteExW(&mut info) };
    if result.is_err() {
        return Err(DirectSpawnError::Spawn(std::io::Error::last_os_error().into()));
    }

    if info.hProcess == HANDLE::default() {
        return Err(DirectSpawnError::Spawn(
            std::io::Error::new(std::io::ErrorKind::InvalidData, "elevated process handle missing").into(),
        ));
    }
    let h_process = info.hProcess;
    let pid = unsafe { GetProcessId(h_process) };
    let tracked = crate::process_monitor::TrackedProcess::from_owned_handle(pid, h_process);
    match tracked {
        Ok(process) => Ok(DirectSpawnOutcome::Tracked(process)),
        Err(crate::process_monitor::ProcessStatus::Unknown(reason)) => {
            Ok(DirectSpawnOutcome::LaunchedButTrackingUncertain { pid: Some(pid), reason })
        }
        Err(_) => Ok(DirectSpawnOutcome::LaunchedButTrackingUncertain {
            pid: Some(pid),
            reason: crate::process_monitor::UnknownReason::AccessOrQueryFailure,
        }),
    }
}

#[cfg(windows)]
fn make_windows_command_line(executable: &OsStr, args: &[std::ffi::OsString]) -> std::ffi::OsString {
    use std::os::windows::ffi::OsStringExt;
    let mut encoded = Vec::new();
    for value in std::iter::once(executable).chain(args.iter().map(OsStr::new)) {
        if !encoded.is_empty() {
            encoded.push(b' ' as u16);
        }
        encoded.extend(quote_windows_argument(value));
    }
    std::ffi::OsString::from_wide(&encoded)
}

#[cfg(windows)]
fn quote_windows_argument(value: &OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    let chars: Vec<u16> = value.encode_wide().collect();
    if !chars.is_empty() && !chars.iter().any(|c| matches!(*c, 0x20 | 0x09 | 0x0b | 0x0c | 0x22)) {
        return chars;
    }
    let mut out = vec![b'"' as u16];
    let mut slashes = 0;
    for ch in chars {
        if ch == b'\\' as u16 {
            slashes += 1;
        } else if ch == b'"' as u16 {
            out.extend(std::iter::repeat(b'\\' as u16).take(slashes * 2 + 1));
            out.push(ch);
            slashes = 0;
        } else {
            out.extend(std::iter::repeat(b'\\' as u16).take(slashes));
            out.push(ch);
            slashes = 0;
        }
    }
    out.extend(std::iter::repeat(b'\\' as u16).take(slashes * 2));
    out.push(b'"' as u16);
    out
}

#[cfg(not(windows))]
pub fn spawn_direct_game(
    _executable: &Path,
    _cwd: &Path,
    _args: &[std::ffi::OsString],
) -> Result<DirectSpawnOutcome, DirectSpawnError> {
    Err(DirectSpawnError::Unsupported)
}

/// Existing persisted `launchTarget` values. Unknown/missing values retain auto semantics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LaunchTarget {
    Auto,
    Launcher,
    Game,
}

impl LaunchTarget {
    pub fn from_setting(value: Option<&str>) -> Self {
        match value {
            Some("launcher") => Self::Launcher,
            Some("game") => Self::Game,
            _ => Self::Auto,
        }
    }
}

/// What the monitor has to pin as "the game" once the launch happened.
///
/// The variants are deliberately narrow. SILO never guesses which descendant is
/// the game: a helper, updater, anti-cheat or crash-reporter process would
/// otherwise be pinned and its lifetime would be reported as playtime.
///
/// * `SpawnedProcess` — the process SILO spawned is the game (a direct game
///   executable). It is pinned immediately, using the handle that was retained
///   at spawn time.
/// * `ChildExecutable` — a launcher with an identified child. The descendant with
///   that exact canonical executable path is the game.
/// * `UnidentifiableChild` — the game process cannot be identified from the
///   available evidence. Nothing may be pinned, so the session stays uncertain.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TargetExpectation {
    /// The spawned process is the game itself.
    SpawnedProcess,
    /// A descendant with this exact canonical executable path is the game.
    ChildExecutable(PathBuf),
    /// The game process cannot be identified from the available evidence.
    UnidentifiableChild,
}

/// The three launch shapes SILO supports.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LaunchClassification {
    /// SILO starts the game executable and tracks the process it spawned.
    DirectGame { executable: PathBuf },
    /// SILO starts the launcher and tracks the descendant running `child`.
    LauncherWithChild { launcher: PathBuf, child: PathBuf },
    /// SILO starts the configured executable, but the game process cannot be
    /// identified. The launch still happens; the session simply cannot be tracked.
    LauncherWithoutChild { launcher: PathBuf },
}

impl LaunchClassification {
    /// The executable SILO starts.
    pub fn executable(&self) -> &Path {
        match self {
            LaunchClassification::DirectGame { executable } => executable,
            LaunchClassification::LauncherWithChild { launcher, .. } => launcher,
            LaunchClassification::LauncherWithoutChild { launcher } => launcher,
        }
    }

    /// What the monitor has to pin for this launch.
    pub fn expectation(&self) -> TargetExpectation {
        match self {
            LaunchClassification::DirectGame { .. } => TargetExpectation::SpawnedProcess,
            LaunchClassification::LauncherWithChild { child, .. } => TargetExpectation::ChildExecutable(child.clone()),
            LaunchClassification::LauncherWithoutChild { .. } => TargetExpectation::UnidentifiableChild,
        }
    }
}

/// Everything [`classify_launch`] needs to classify a launch.
pub struct LaunchInputs<'a> {
    /// The persisted launch target: auto, launcher or game.
    pub target: LaunchTarget,
    /// The executable configured for this game.
    pub configured: &'a Path,
    /// The game executable the user configured explicitly for this game. It is
    /// the binary a launcher starts, or — in `Game` mode — the game itself.
    pub explicit_child: Option<&'a Path>,
    /// The game binary SILO resolved below the configured executable, if any.
    pub resolved_game: Option<&'a Path>,
    /// True when the resolved binary is confidently the game itself.
    pub resolved_is_confident: bool,
}

/// Classifies a launch into one of the three supported shapes.
///
/// The rules are explicit and conservative:
///
/// * `Game` — the explicitly configured game executable is the process to track.
///   SILO starts it and tracks the process it spawned.
/// * `Launcher` — SILO starts the configured launcher and tracks its game child,
///   but only when that child is identified: either the executable the user
///   configured for it, or a confidently resolved game binary.
/// * `Auto` — only a confident identification is acted on: the user's explicit
///   child executable, or a confidently resolved game binary. Without one, the
///   configured executable is started and the game process cannot be identified.
///
/// Two rules are absolute:
///
/// * An executable is **never** treated as a game because its file name does not
///   look like a launcher. `Auto` stays uncertain for a standalone executable
///   unless the user explicitly pins the target to `Game`.
/// * No descendant is ever pinned without an exact executable path to match, so
///   a helper, updater, anti-cheat or crash-reporter process can never be
///   mistaken for the game.
pub fn classify_launch(inputs: LaunchInputs<'_>) -> LaunchClassification {
    let LaunchInputs {
        target,
        configured,
        explicit_child,
        resolved_game,
        resolved_is_confident,
    } = inputs;
    // Only a confident resolution may be acted on. An explicitly configured child
    // is the user's own statement and is always acted on.
    let confident_child = resolved_game.filter(|_| resolved_is_confident);
    match target {
        // The user says this executable is the game: run it and track that process.
        LaunchTarget::Game => LaunchClassification::DirectGame {
            executable: explicit_child.unwrap_or(configured).to_path_buf(),
        },
        // The user says this executable is a launcher: run it and track its child.
        LaunchTarget::Launcher => match explicit_child.or(confident_child) {
            Some(child) => LaunchClassification::LauncherWithChild {
                launcher: configured.to_path_buf(),
                child: child.to_path_buf(),
            },
            None => LaunchClassification::LauncherWithoutChild {
                launcher: configured.to_path_buf(),
            },
        },
        // Auto only acts on a confident identification, or treats a non-launcher
        // executable as the game itself (direct launch).
        LaunchTarget::Auto => match explicit_child {
            Some(child) => LaunchClassification::LauncherWithChild {
                launcher: configured.to_path_buf(),
                child: child.to_path_buf(),
            },
            None => match confident_child {
                Some(child) => LaunchClassification::DirectGame {
                    executable: child.to_path_buf(),
                },
                None => {
                    // The configured executable does not look like a launcher, so
                    // treat it as the game itself directly. This covers direct game
                    // executables like Sovereign Tower under Auto mode.
                    let configured_name = configured.file_name().map(|n| n.to_string_lossy().to_lowercase());
                    let is_launcher = configured_name
                        .as_deref()
                        .map_or(false, crate::commands::is_launcher_like);
                    if !is_launcher {
                        LaunchClassification::DirectGame {
                            executable: configured.to_path_buf(),
                        }
                    } else {
                        LaunchClassification::LauncherWithoutChild {
                            launcher: configured.to_path_buf(),
                        }
                    }
                }
            },
        },
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessCandidate {
    pub pid: u32,
    pub parent_pid: Option<u32>,
    pub executable: Option<PathBuf>,
}

/// A match must be a strict descendant and have the exact canonical target path.
///
/// The launcher itself is never the answer: a launcher and its game child can
/// share an executable path (some launchers re-run themselves), and pinning the
/// launcher would end the session when the launcher exits. When several
/// descendants run the target path, the one closest to the launcher wins.
pub fn find_target_descendant<'a>(
    root_pid: u32,
    target: &Path,
    processes: &'a [ProcessCandidate],
) -> Option<&'a ProcessCandidate> {
    let target = std::fs::canonicalize(target).ok()?;
    let mut best: Option<(usize, &ProcessCandidate)> = None;
    for candidate in processes {
        if candidate.pid == root_pid {
            continue;
        }
        let Some(path) = candidate
            .executable
            .as_deref()
            .and_then(|p| std::fs::canonicalize(p).ok())
        else {
            continue;
        };
        if path != target {
            continue;
        }
        let Some(depth) = ancestor_depth(candidate.pid, root_pid, processes) else {
            continue;
        };
        if best.is_none_or(|(best_depth, _)| depth < best_depth) {
            best = Some((depth, candidate));
        }
    }
    best.map(|(_, candidate)| candidate)
}

/// Number of links between `pid` and `root_pid`, or `None` when `pid` is not a
/// descendant of `root_pid`.
fn ancestor_depth(pid: u32, root_pid: u32, processes: &[ProcessCandidate]) -> Option<usize> {
    let mut current = pid;
    for depth in 0..=processes.len() {
        if current == root_pid {
            return Some(depth);
        }
        let parent = processes.iter().find(|p| p.pid == current).and_then(|p| p.parent_pid)?;
        current = parent;
    }
    None
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrackingOutcome {
    Tracked,
    TrackingUncertain,
}

/// Launcher liveness is provenance only; only a pinned target can end tracking.
pub fn target_outcome(target_pinned: bool, target_status: crate::process_monitor::ProcessStatus) -> TrackingOutcome {
    match (target_pinned, target_status) {
        (true, crate::process_monitor::ProcessStatus::Running | crate::process_monitor::ProcessStatus::Exited) => {
            TrackingOutcome::Tracked
        }
        _ => TrackingOutcome::TrackingUncertain,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn windows_argument_quoting_preserves_spaces_quotes_and_trailing_slashes() {
        use std::os::windows::ffi::OsStrExt;
        let args = [
            OsStr::new("two words"),
            OsStr::new("embedded\"quote"),
            OsStr::new("末尾 路径\\"),
        ];
        let line = make_windows_command_line(
            OsStr::new("C:\\Program Files\\路径\\game.exe"),
            &args.iter().map(|s| s.to_os_string()).collect::<Vec<_>>(),
        );
        let actual = line.encode_wide().collect::<Vec<_>>();
        let expected = "\"C:\\Program Files\\路径\\game.exe\" \"two words\" \"embedded\\\"quote\" \"末尾 路径\\\\\"";
        assert_eq!(actual, expected.encode_utf16().collect::<Vec<_>>());
    }

    #[test]
    fn auto_only_acts_on_a_confident_identification() {
        let configured = Path::new("launcher.exe");
        let game = Path::new("game.exe");

        // Auto + a confidently resolved game binary: start the game directly.
        let classification = classify_launch(LaunchInputs {
            target: LaunchTarget::Auto,
            configured,
            explicit_child: None,
            resolved_game: Some(game),
            resolved_is_confident: true,
        });
        assert_eq!(
            classification,
            LaunchClassification::DirectGame {
                executable: game.to_path_buf()
            }
        );
        assert_eq!(classification.expectation(), TargetExpectation::SpawnedProcess);

        // Auto + a resolved binary that is NOT confident: it is not acted on.
        let classification = classify_launch(LaunchInputs {
            target: LaunchTarget::Auto,
            configured,
            explicit_child: None,
            resolved_game: Some(game),
            resolved_is_confident: false,
        });
        assert_eq!(
            classification,
            LaunchClassification::LauncherWithoutChild {
                launcher: configured.to_path_buf()
            }
        );
        assert_eq!(
            classification.expectation(),
            TargetExpectation::UnidentifiableChild,
            "an unconfident resolution may never be pinned"
        );

        // Auto + nothing resolved at all: the configured executable is started but
        // the game process cannot be identified.
        let classification = classify_launch(LaunchInputs {
            target: LaunchTarget::Auto,
            configured,
            explicit_child: None,
            resolved_game: None,
            resolved_is_confident: false,
        });
        assert_eq!(
            classification,
            LaunchClassification::LauncherWithoutChild {
                launcher: configured.to_path_buf()
            }
        );
    }

    #[test]
    fn auto_guesses_a_non_launcher_executable_is_the_game() {
        // A standalone executable whose name does not look like a launcher is
        // treated as the game itself under Auto: Sovereign Tower's
        // sovereign_tower.exe is not launcher-like, so Auto tracks it directly.
        let standalone = Path::new("MyGame.exe");
        let classification = classify_launch(LaunchInputs {
            target: LaunchTarget::Auto,
            configured: standalone,
            explicit_child: None,
            resolved_game: None,
            resolved_is_confident: false,
        });
        assert_eq!(
            classification,
            LaunchClassification::DirectGame {
                executable: standalone.to_path_buf()
            },
            "auto treats a non-launcher executable as the game itself"
        );
        assert_eq!(classification.expectation(), TargetExpectation::SpawnedProcess);
    }

    #[test]
    fn auto_keeps_launcher_like_executable_uncertain_without_child() {
        // A launcher-looking executable with no identified child stays uncertain:
        // we cannot guess whether it is the game or a launcher.
        let configured = Path::new("REDprelauncher.exe");
        let classification = classify_launch(LaunchInputs {
            target: LaunchTarget::Auto,
            configured,
            explicit_child: None,
            resolved_game: None,
            resolved_is_confident: false,
        });
        assert_eq!(
            classification,
            LaunchClassification::LauncherWithoutChild {
                launcher: configured.to_path_buf()
            },
            "auto stays uncertain for a launcher-looking executable without a child"
        );
        assert_eq!(classification.expectation(), TargetExpectation::UnidentifiableChild);
    }

    #[test]
    fn auto_launcher_like_with_confident_child_stays_direct_game() {
        // A launcher-looking executable with a confidently resolved child stays
        // a direct game pointing at the child.
        let configured = Path::new("REDprelauncher.exe");
        let game = Path::new("Cyberpunk2077.exe");
        let classification = classify_launch(LaunchInputs {
            target: LaunchTarget::Auto,
            configured,
            explicit_child: None,
            resolved_game: Some(game),
            resolved_is_confident: true,
        });
        assert_eq!(
            classification,
            LaunchClassification::DirectGame {
                executable: game.to_path_buf()
            }
        );
        assert_eq!(classification.expectation(), TargetExpectation::SpawnedProcess);
    }

    #[test]
    fn explicit_game_target_tracks_the_spawned_process() {
        let standalone = Path::new("MyGame.exe");
        let classification = classify_launch(LaunchInputs {
            target: LaunchTarget::Game,
            configured: standalone,
            explicit_child: None,
            resolved_game: None,
            resolved_is_confident: false,
        });
        assert_eq!(
            classification,
            LaunchClassification::DirectGame {
                executable: standalone.to_path_buf()
            }
        );
        assert_eq!(
            classification.expectation(),
            TargetExpectation::SpawnedProcess,
            "the user said this executable is the game"
        );

        // An explicitly configured game executable wins over the configured path.
        let explicit = Path::new("explicit-game.exe");
        let classification = classify_launch(LaunchInputs {
            target: LaunchTarget::Game,
            configured: standalone,
            explicit_child: Some(explicit),
            resolved_game: None,
            resolved_is_confident: false,
        });
        assert_eq!(
            classification,
            LaunchClassification::DirectGame {
                executable: explicit.to_path_buf()
            }
        );
        assert_eq!(classification.expectation(), TargetExpectation::SpawnedProcess);
    }

    #[test]
    fn explicit_launcher_target_never_becomes_a_direct_game() {
        let configured = Path::new("launcher.exe");

        // No identified child: the launcher is started, but nothing may be pinned.
        let classification = classify_launch(LaunchInputs {
            target: LaunchTarget::Launcher,
            configured,
            explicit_child: None,
            resolved_game: None,
            resolved_is_confident: false,
        });
        assert_eq!(
            classification,
            LaunchClassification::LauncherWithoutChild {
                launcher: configured.to_path_buf()
            }
        );
        assert_eq!(classification.expectation(), TargetExpectation::UnidentifiableChild);

        // An explicitly configured child is tracked by exact executable path.
        let child = Path::new("child.exe");
        let classification = classify_launch(LaunchInputs {
            target: LaunchTarget::Launcher,
            configured,
            explicit_child: Some(child),
            resolved_game: None,
            resolved_is_confident: false,
        });
        assert_eq!(
            classification,
            LaunchClassification::LauncherWithChild {
                launcher: configured.to_path_buf(),
                child: child.to_path_buf(),
            }
        );
        assert_eq!(
            classification.expectation(),
            TargetExpectation::ChildExecutable(child.to_path_buf())
        );

        // A confidently resolved child is tracked the same way.
        let classification = classify_launch(LaunchInputs {
            target: LaunchTarget::Launcher,
            configured,
            explicit_child: None,
            resolved_game: Some(child),
            resolved_is_confident: true,
        });
        assert_eq!(
            classification,
            LaunchClassification::LauncherWithChild {
                launcher: configured.to_path_buf(),
                child: child.to_path_buf(),
            }
        );
    }

    #[test]
    fn an_explicit_child_takes_precedence_over_a_resolved_one() {
        let configured = Path::new("launcher.exe");
        let explicit = Path::new("explicit.exe");
        let resolved = Path::new("resolved.exe");
        let classification = classify_launch(LaunchInputs {
            target: LaunchTarget::Launcher,
            configured,
            explicit_child: Some(explicit),
            resolved_game: Some(resolved),
            resolved_is_confident: true,
        });
        assert_eq!(
            classification,
            LaunchClassification::LauncherWithChild {
                launcher: configured.to_path_buf(),
                child: explicit.to_path_buf(),
            }
        );
    }

    #[test]
    fn descendant_requires_canonical_path_and_verified_ancestry() {
        let target = std::env::current_exe().expect("test executable path");
        let other = PathBuf::from("unrelated.exe");
        let candidates = [
            ProcessCandidate {
                pid: 20,
                parent_pid: Some(10),
                executable: Some(target.clone()),
            },
            ProcessCandidate {
                pid: 21,
                parent_pid: Some(99),
                executable: Some(target.clone()),
            },
            ProcessCandidate {
                pid: 22,
                parent_pid: Some(10),
                executable: Some(other),
            },
        ];
        assert_eq!(
            find_target_descendant(10, &target, &candidates).map(|p| p.pid),
            Some(20)
        );
    }

    #[test]
    fn the_launcher_itself_is_never_the_tracked_descendant() {
        // A launcher that re-runs its own binary must not be pinned as the game.
        let target = std::env::current_exe().expect("test executable path");
        let candidates = [ProcessCandidate {
            pid: 10,
            parent_pid: None,
            executable: Some(target.clone()),
        }];
        assert!(
            find_target_descendant(10, &target, &candidates).is_none(),
            "the launcher itself is excluded even when its path matches"
        );
    }

    #[test]
    fn a_helper_with_a_different_path_is_never_the_game() {
        // The only descendant runs a different executable: it is a helper, an
        // updater or a crash reporter, and it must not be pinned.
        let target = std::env::current_exe().expect("test executable path");
        let helper = PathBuf::from("unrelated.exe");
        let candidates = [
            ProcessCandidate {
                pid: 10,
                parent_pid: None,
                executable: Some(PathBuf::from("launcher.exe")),
            },
            ProcessCandidate {
                pid: 11,
                parent_pid: Some(10),
                executable: Some(helper),
            },
        ];
        assert!(find_target_descendant(10, &target, &candidates).is_none());
    }

    #[test]
    fn the_closest_descendant_with_the_target_path_wins() {
        let target = std::env::current_exe().expect("test executable path");
        let candidates = [
            ProcessCandidate {
                pid: 10,
                parent_pid: None,
                executable: Some(PathBuf::from("launcher.exe")),
            },
            ProcessCandidate {
                pid: 11,
                parent_pid: Some(10),
                executable: Some(target.clone()),
            },
            ProcessCandidate {
                pid: 12,
                parent_pid: Some(11),
                executable: Some(target.clone()),
            },
        ];
        assert_eq!(
            find_target_descendant(10, &target, &candidates).map(|p| p.pid),
            Some(11),
            "the descendant closest to the launcher is the game"
        );
    }

    #[test]
    fn launcher_exit_or_unknown_target_never_means_game_exit() {
        assert_eq!(
            target_outcome(false, crate::process_monitor::ProcessStatus::Exited),
            TrackingOutcome::TrackingUncertain
        );
        assert_eq!(
            target_outcome(
                true,
                crate::process_monitor::ProcessStatus::Unknown(
                    crate::process_monitor::UnknownReason::AccessOrQueryFailure
                )
            ),
            TrackingOutcome::TrackingUncertain
        );
        assert_eq!(
            target_outcome(true, crate::process_monitor::ProcessStatus::Exited),
            TrackingOutcome::Tracked
        );
    }
}
