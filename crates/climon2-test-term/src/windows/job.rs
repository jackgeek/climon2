//! The job object a host may put its shell in: a harness setting of its own,
//! separate from the host model (docs/windows-qualification.md, "Host models").

use std::{
    io,
    os::windows::io::{FromRawHandle, OwnedHandle},
};
use windows_sys::Win32::System::JobObjects::{
    CreateJobObjectW, JOB_OBJECT_LIMIT_BREAKAWAY_OK, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject,
};

/// Whether the shell starts in a job, and whether that job lets a process
/// start outside it (`CREATE_BREAKAWAY_FROM_JOB`). A job the harness creates
/// ends every process still in it when the terminal is dropped.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum JobShape {
    /// No job of the harness's: the shell is in whatever job the test is.
    #[default]
    None,
    /// A job with `JOB_OBJECT_LIMIT_BREAKAWAY_OK`.
    AllowsBreakaway,
    /// A job without it: starting a process outside it fails.
    RefusesBreakaway,
}

impl JobShape {
    fn limits(self) -> Option<u32> {
        match self {
            Self::None => None,
            Self::AllowsBreakaway => {
                Some(JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_BREAKAWAY_OK)
            }
            Self::RefusesBreakaway => Some(JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE),
        }
    }

    /// The job to start the shell in, if this shape has one.
    pub(crate) fn create(self) -> io::Result<Option<OwnedHandle>> {
        let Some(limits) = self.limits() else {
            return Ok(None);
        };
        // An anonymous job with default security.
        let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if raw.is_null() {
            return Err(io::Error::last_os_error());
        }
        // `raw` is a new job handle that nothing else owns.
        let job = unsafe { OwnedHandle::from_raw_handle(raw) };
        let mut information = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        information.BasicLimitInformation.LimitFlags = limits;
        // The call reads one JOBOBJECT_EXTENDED_LIMIT_INFORMATION of the size given.
        let set = unsafe {
            SetInformationJobObject(
                raw,
                JobObjectExtendedLimitInformation,
                (&raw const information).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if set == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Some(job))
    }
}
