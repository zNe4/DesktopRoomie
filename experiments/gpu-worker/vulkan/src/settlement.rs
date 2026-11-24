//! Independent evidence, never inferred from slot retirement or process exit.
use roomie_worker_protocol::JobKey;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Operation {
    pub session: u64,
    pub job: JobKey,
    pub phase: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Submission {
    NotAttempted,
    Attempted,
    IndexReturned,
    Accepted,
    Complete,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mapping {
    NotRequested,
    Requested,
    Confirmed,
    Failed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    NeverOpened,
    Live,
    Released,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Available,
    InFlight,
    Retired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Evidence {
    pub operation: Operation,
    pub submission: Submission,
    pub mapping: Mapping,
    pub view: View,
    pub unmapped: bool,
    pub retired: bool,
    /// A native poll panic can invalidate otherwise completed evidence.
    pub uncertain: bool,
}
impl Evidence {
    pub fn new(operation: Operation) -> Self {
        Self {
            operation,
            submission: Submission::NotAttempted,
            mapping: Mapping::NotRequested,
            view: View::NeverOpened,
            unmapped: false,
            retired: false,
            uncertain: false,
        }
    }
    pub fn submission_settled(self) -> bool {
        matches!(
            self.submission,
            Submission::NotAttempted | Submission::Complete
        ) && !self.uncertain
    }
    pub fn staging_settled(self) -> bool {
        !self.uncertain
            && self.view != View::Live
            && match self.mapping {
                Mapping::NotRequested => self.view == View::NeverOpened,
                Mapping::Requested => false,
                Mapping::Confirmed | Mapping::Failed => self.unmapped,
            }
    }
    pub fn settled(self) -> bool {
        self.submission_settled() && self.staging_settled()
    }
    pub fn safely_retired(self) -> bool {
        self.retired && self.settled()
    }
    pub fn frame_ready(self) -> bool {
        self.settled()
            && self.submission == Submission::Complete
            && self.mapping == Mapping::Confirmed
            && self.view == View::Released
            && self.unmapped
            && !self.retired
    }
    pub fn rejected_before_submission(self) -> bool {
        self.settled()
            && self.submission == Submission::NotAttempted
            && self.mapping == Mapping::NotRequested
            && self.view == View::NeverOpened
            && !self.unmapped
    }
}
