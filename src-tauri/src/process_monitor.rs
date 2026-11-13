//! Conservative process identity and liveness primitives.

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub creation_filetime: u64,
    pub executable: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProcessStatus {
    Running,
    Exited,
    Unknown(UnknownReason),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UnknownReason {
    Unsupported,
    AccessOrQueryFailure,
    WaitFailure,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PersistedIdentityMatch {
    SameProcess,
    DifferentProcess,
}

pub const fn combine_filetime(high: u32, low: u32) -> u64 {
    ((high as u64) << 32) | low as u64
}

pub const fn split_filetime(value: u64) -> (u32, u32) {
    ((value >> 32) as u32, value as u32)
}

pub const fn compare_persisted_identity(pid_matches: bool, expected: u64, observed: u64) -> PersistedIdentityMatch {
    if pid_matches && expected == observed {
        PersistedIdentityMatch::SameProcess
    } else {
        PersistedIdentityMatch::DifferentProcess
    }
}

#[cfg(windows)]
mod platform {
    use super::{
        combine_filetime, compare_persisted_identity, PersistedIdentityMatch, ProcessIdentity, ProcessStatus,
        UnknownReason,
    };
    use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
    use windows::Win32::System::Threading::{
        GetExitCodeProcess, GetPriorityClass, GetProcessTimes, OpenProcess, QueryFullProcessImageNameW,
        WaitForSingleObject, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    };

    struct OwnedProcessHandle(HANDLE);

    impl Drop for OwnedProcessHandle {
        fn drop(&mut self) {
            // SAFETY: this wrapper exclusively owns the valid handle returned by OpenProcess.
            let _ = unsafe { CloseHandle(self.0) };
        }
    }

    pub struct TrackedProcess {
        pub identity: ProcessIdentity,
        handle: OwnedProcessHandle,
    }

    impl TrackedProcess {
        pub(crate) fn from_owned_handle(pid: u32, handle: HANDLE) -> Result<Self, ProcessStatus> {
            let handle = OwnedProcessHandle(handle);
            let mut created = FILETIME::default();
            let mut exited = FILETIME::default();
            let mut kernel = FILETIME::default();
            let mut user = FILETIME::default();
            // SAFETY: output pointers are valid and handle is owned for this call.
            unsafe { GetProcessTimes(handle.0, &mut created, &mut exited, &mut kernel, &mut user) }
                .map_err(|_| ProcessStatus::Unknown(UnknownReason::AccessOrQueryFailure))?;
            let creation_filetime = combine_filetime(created.dwHighDateTime, created.dwLowDateTime);
            let executable = query_image(handle.0);
            Ok(Self {
                identity: ProcessIdentity {
                    pid,
                    creation_filetime,
                    executable,
                },
                handle,
            })
        }
    }

    impl TrackedProcess {
        pub fn open(pid: u32) -> Result<Self, ProcessStatus> {
            // SAFETY: OpenProcess is called with a PID and no inherited handle; returned handle is owned here.
            let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE, false, pid) }
                .map_err(|error| {
                    if error.code().0 as u32 == 0x8007_0057 {
                        ProcessStatus::Exited
                    } else {
                        ProcessStatus::Unknown(UnknownReason::AccessOrQueryFailure)
                    }
                })?;
            Self::from_owned_handle(pid, handle)
        }

        pub fn poll(&self) -> ProcessStatus {
            // SAFETY: retained handle remains valid for this call; zero timeout is a nonblocking observation.
            match unsafe { WaitForSingleObject(self.handle.0, 0) } {
                WAIT_OBJECT_0 => ProcessStatus::Exited,
                WAIT_TIMEOUT => ProcessStatus::Running,
                _ => ProcessStatus::Unknown(UnknownReason::WaitFailure),
            }
        }

        /// Returns the Windows process exit code after the retained handle is signaled.
        pub fn exit_code(&self) -> Result<Option<u32>, ProcessStatus> {
            match self.poll() {
                ProcessStatus::Running => return Ok(None),
                ProcessStatus::Unknown(reason) => return Err(ProcessStatus::Unknown(reason)),
                ProcessStatus::Exited => {}
            }
            let mut code = 0;
            // SAFETY: retained process handle is valid and `code` is writable.
            unsafe { GetExitCodeProcess(self.handle.0, &mut code) }
                .map_err(|_| ProcessStatus::Unknown(UnknownReason::AccessOrQueryFailure))?;
            Ok(Some(code))
        }

        pub fn priority_class(&self) -> Result<u32, ProcessStatus> {
            // SAFETY: retained process handle is valid for GetPriorityClass.
            let priority = unsafe { GetPriorityClass(self.handle.0) };
            if priority == 0 {
                Err(ProcessStatus::Unknown(UnknownReason::AccessOrQueryFailure))
            } else {
                Ok(priority)
            }
        }
    }

    fn query_image(handle: HANDLE) -> Option<String> {
        let mut buffer = vec![0u16; 32_768];
        let mut length = buffer.len() as u32;
        // SAFETY: buffer is writable and length describes its capacity, as required by the API.
        unsafe {
            QueryFullProcessImageNameW(
                handle,
                PROCESS_NAME_FORMAT(0),
                windows::core::PWSTR(buffer.as_mut_ptr()),
                &mut length,
            )
        }
        .ok()?;
        String::from_utf16(&buffer[..length as usize]).ok()
    }

    pub fn reopen_persisted(pid: u32, creation_filetime: u64) -> (ProcessStatus, Option<TrackedProcess>) {
        match TrackedProcess::open(pid) {
            Ok(tracked) => {
                let matches = compare_persisted_identity(true, creation_filetime, tracked.identity.creation_filetime);
                match matches {
                    PersistedIdentityMatch::SameProcess => (tracked.poll(), Some(tracked)),
                    PersistedIdentityMatch::DifferentProcess => (ProcessStatus::Exited, None),
                }
            }
            Err(status) => (status, None),
        }
    }

    pub use TrackedProcess as PlatformTrackedProcess;
}

#[cfg(windows)]
pub use platform::{reopen_persisted, PlatformTrackedProcess as TrackedProcess};

#[cfg(not(windows))]
pub struct TrackedProcess {
    /// Present only so shared call sites compile. Process tracking is not
    /// implemented on this platform: `open` always fails, so no value of this
    /// type can ever exist and this identity is never populated or observed.
    pub identity: ProcessIdentity,
}

#[cfg(not(windows))]
impl TrackedProcess {
    pub fn open(_pid: u32) -> Result<Self, ProcessStatus> {
        Err(ProcessStatus::Unknown(UnknownReason::Unsupported))
    }

    pub fn poll(&self) -> ProcessStatus {
        ProcessStatus::Unknown(UnknownReason::Unsupported)
    }

    /// Unsupported platform: no process can be tracked, so no exit code can
    /// ever be observed.
    pub fn exit_code(&self) -> Result<Option<u32>, ProcessStatus> {
        Err(ProcessStatus::Unknown(UnknownReason::Unsupported))
    }
}

#[cfg(not(windows))]
pub fn reopen_persisted(_pid: u32, _creation_filetime: u64) -> (ProcessStatus, Option<TrackedProcess>) {
    (ProcessStatus::Unknown(UnknownReason::Unsupported), None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filetime_round_trips_across_halves() {
        for value in [0, 1, u32::MAX as u64, 0x1234_5678_9abc_def0, u64::MAX] {
            let (high, low) = split_filetime(value);
            assert_eq!(combine_filetime(high, low), value);
        }
    }

    #[test]
    fn persisted_identity_requires_pid_and_creation_time_match() {
        assert_eq!(
            compare_persisted_identity(true, 42, 42),
            PersistedIdentityMatch::SameProcess
        );
        assert_eq!(
            compare_persisted_identity(false, 42, 42),
            PersistedIdentityMatch::DifferentProcess
        );
        assert_eq!(
            compare_persisted_identity(true, 42, 43),
            PersistedIdentityMatch::DifferentProcess
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn unsupported_platform_is_explicitly_unknown() {
        assert_eq!(
            TrackedProcess::open(1).err(),
            Some(ProcessStatus::Unknown(UnknownReason::Unsupported))
        );
        assert_eq!(
            reopen_persisted(1, 1).0,
            ProcessStatus::Unknown(UnknownReason::Unsupported)
        );
    }
}
