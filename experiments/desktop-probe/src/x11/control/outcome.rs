use crate::control::{self, Command, ExitCode, Lifecycle, Reason, Stage, Status, TerminalResponse};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallerOutcome {
    NoInstance,
    Protocol,
    InvalidDestination(String),
    Lifecycle(Lifecycle),
    Terminal(TerminalResponse),
    Unknown(&'static str),
}
impl CallerOutcome {
    pub fn exit_code(&self) -> ExitCode {
        match self {
            Self::NoInstance => ExitCode::NoInstance,
            Self::Protocol | Self::InvalidDestination(_) => ExitCode::Protocol,
            Self::Lifecycle(_) => ExitCode::Busy,
            Self::Terminal(response) => response.status.exit_code(),
            Self::Unknown(_) => ExitCode::OutcomeUnknown,
        }
    }
    pub fn diagnostic(&self, command: Command) -> String {
        let message = match self {
            Self::NoInstance => "no running instance".into(),
            Self::Protocol => "incompatible, stale, or malformed owner endpoint".into(),
            Self::InvalidDestination(reason) => format!("{reason}; request not sent"),
            Self::Lifecycle(lifecycle) => format!("owner is {lifecycle:?}"),
            Self::Terminal(response) if command == Command::BringTop => {
                bring_top_diagnostic(response)
            }
            Self::Terminal(response) if command == Command::BringHere => {
                bring_here_diagnostic(response)
            }
            Self::Terminal(response) if command == Command::Recover => recover_diagnostic(response),
            Self::Terminal(response)
                if command == Command::Hide && response.status == Status::Success =>
            {
                "confirmed minimized; existing instance remains running".into()
            }
            Self::Terminal(response)
                if command == Command::Show && response.status == Status::Success =>
            {
                let action = if response.detail.progress.host_mutation_dispatched {
                    "restored"
                } else {
                    "already not minimized"
                };
                if response.detail.progress.placement_deferred {
                    format!("{action} on another workspace; original workspace/layer preserved; placement deferred, input unavailable; not necessarily visible here")
                } else {
                    format!("{action} here; original workspace/layer preserved; placement verified, input ready")
                }
            }
            Self::Terminal(response)
                if command == Command::Show
                    && response.detail.reason == Reason::ConfirmationTimeout =>
            {
                "failed — confirmation timed out; final observations did not prove Show readiness"
                    .into()
            }
            Self::Terminal(response)
                if command == Command::Show && response.detail.reason == Reason::LayoutChanged =>
            {
                "failed — workspace/layout preservation could not be confirmed".into()
            }
            Self::Terminal(response)
                if command == Command::Show && response.detail.reason == Reason::LayerChanged =>
            {
                "failed — effective layer changed".into()
            }
            Self::Terminal(response) => format!(
                "{:?} — {:?} ({:?})",
                response.status, response.detail.reason, response.detail.stage
            ),
            Self::Unknown(reason) => format!("outcome unknown — {reason}"),
        };
        format!("{command}: {message}")
    }
}

fn recover_diagnostic(response: &TerminalResponse) -> String {
    let d = response.detail;
    let reason = match d.reason {
        Reason::None => "no failure reported".into(),
        Reason::ConfirmationTimeout => "confirmation timed out".into(),
        Reason::LayoutChanged => "frozen workspace/layout changed".into(),
        Reason::LayerChanged => "effective layer changed incompatibly".into(),
        Reason::OwnershipLost => "owner selection lost".into(),
        Reason::X11Failure => "native X11 failure".into(),
        Reason::Closing => "owner is closing".into(),
        Reason::BodyDestroyed => "existing body was destroyed".into(),
        Reason::Withdrawn => "existing body was withdrawn".into(),
        Reason::Unverifiable => "required state could not be verified".into(),
        Reason::Unsupported => "required WM support unavailable".into(),
        Reason::Busy => "another remote or local layer/placement operation is active".into(),
        reason => format!("{reason:?}"),
    };
    let history = d.placement.history;
    match response.status {
        Status::Success => {
            let transitions = match (history.relocation_confirmed, history.restoration_confirmed) {
                (true, true) => "; relocation and restoration confirmed earlier",
                (true, false) => "; relocation confirmed earlier",
                (false, true) => "; restoration confirmed earlier",
                (false, false) => "",
            };
            format!("same existing body confirmed on caller's frozen workspace, not minimized and Above; placement verified, input ready{transitions}")
        }
        Status::Partial => {
            let transitions = match (history.relocation_confirmed, history.restoration_confirmed) {
                (true, true) => "relocation and restoration were both confirmed earlier",
                (true, false) => "relocation was confirmed earlier; restoration was not confirmed as history",
                (false, true) => "restoration was confirmed earlier; relocation was not confirmed as history",
                (false, false) => "neither relocation nor restoration was confirmed as history",
            };
            format!("partial — {transitions}; complete final Recover postcondition is not currently confirmed; {reason} ({:?})", d.stage)
        }
        Status::Failed => format!("failed — complete Recover was not confirmed; neither relocation nor restoration was confirmed as history; {reason} ({:?}){}", d.stage,
            if d.progress.host_mutation_dispatched { "; a native request may have reached the WM and later state may still change" } else { "" }),
        status => format!("{status:?} — Recover was not completed; {reason} ({:?})", d.stage),
    }
}

fn bring_here_diagnostic(response: &TerminalResponse) -> String {
    let d = response.detail;
    let reason = match d.reason {
        Reason::LayoutChanged => "workspace/layout changed",
        Reason::LayerChanged => "effective layer changed",
        Reason::ConfirmationTimeout => "confirmation timed out",
        Reason::X11Failure => "X11 failure",
        Reason::OwnershipLost => "owner selection lost",
        Reason::Closing => "owner shutdown",
        Reason::Unverifiable => "state unverifiable",
        Reason::Unsupported => "required WM support unavailable",
        reason => return here_terminal_diagnostic(response, &format!("{reason:?}")),
    };
    here_terminal_diagnostic(response, reason)
}

fn here_terminal_diagnostic(response: &TerminalResponse, reason: &str) -> String {
    let d = response.detail;
    match response.status {
        Status::Success if d.progress.placement_deferred => "existing body confirmed on caller's frozen workspace while remaining genuinely minimized; layer preserved; placement/input deferred".into(),
        Status::Success => format!("existing body {} on caller's frozen workspace; layer and minimization semantics preserved; placement verified, input ready",
            if d.placement.history.relocation_confirmed { "relocated and confirmed" } else { "confirmed" }),
        Status::Partial => format!("partial — actual relocation was confirmed earlier, but the complete final Bring Here postcondition was not retained; {reason}"),
        Status::Failed => format!("failed — complete Bring Here was not confirmed; no accepted relocation history; {reason}{}",
            if d.progress.host_mutation_dispatched { "; native dispatch occurred, so later WM movement remains possible" } else { "" }),
        Status::Busy => "Busy — another remote or local layer/placement operation is active".into(),
        status => format!("{status:?} — {reason} ({:?})", d.stage),
    }
}

fn bring_top_diagnostic(response: &TerminalResponse) -> String {
    use control::{LayerObservation, Visibility};
    let d = response.detail;
    let location =
        if d.visibility == Visibility::NotMinimizedElsewhere || d.progress.placement_deferred {
            "on another workspace"
        } else {
            "here"
        };
    let state = match d.layer {
        LayerObservation::Unknown => "unverified",
        LayerObservation::Normal => "Normal",
        LayerObservation::Above => "Above",
        LayerObservation::Below => "Below",
        LayerObservation::Conflict => "conflicting Above + Below",
    };
    let reason = match d.reason {
        Reason::ConfirmationTimeout => "confirmation timed out".into(),
        Reason::Unsupported => "required WM support unavailable".into(),
        Reason::Unverifiable => "state unverifiable".into(),
        reason => format!("{reason:?}"),
    };
    match response.status {
        Status::Success if !d.progress.host_mutation_dispatched => format!("already not minimized + Above {location}; original workspace preserved; no restoration/layer request needed{}",
            if d.progress.placement_deferred { "; placement deferred, input unavailable" } else { "" }),
        Status::Success => format!("not minimized + Above confirmed {location}; original workspace preserved{}",
            if d.progress.placement_deferred { "; placement deferred, input unavailable" } else { "" }),
        Status::Partial if !d.progress.not_minimized_confirmed => format!("partial — restoration was confirmed earlier; current state is unverified or no longer not minimized ({:?}); {reason}; current effective layer: {state}", d.visibility),
        Status::Partial if d.reason == Reason::LayoutChanged => format!("partial — restoration was confirmed earlier; workspace/layout preservation failed; current body {location}; current effective layer: {state}"),
        Status::Partial if d.progress.above_confirmed => format!("partial — restoration was confirmed; Above is currently confirmed {location}, but the operation failed; {reason}"),
        Status::Partial if d.stage != Stage::Layer => format!("partial — restoration confirmed {location}, but visibility/input readiness failed; {reason}; current effective layer: {state}"),
        Status::Partial => format!("partial — restoration confirmed {location}, but the complete Above contract failed; {reason}; current effective layer: {state}{}",
            if d.progress.placement_deferred { "; original workspace preserved; placement deferred, input unavailable" } else { "" }),
        Status::Failed if d.stage == Stage::Shutdown => format!("failed — operation ended during shutdown; {reason}; current effective layer: {state}"),
        Status::Failed if d.progress.above_confirmed => format!("failed — Above is currently confirmed {location}, but the operation failed; {reason}"),
        Status::Failed if d.stage != Stage::Layer => format!("failed — visibility readiness could not be confirmed before layer work; {reason}; current effective layer: {state}"),
        Status::Failed => format!("failed — Above contract was not confirmed; {reason}; current effective layer: {state}{}",
            if d.visibility == Visibility::NotMinimizedElsewhere { "; on another workspace" } else { "" }),
        Status::Busy => "Busy — another remote or local layer/placement operation is active".into(),
        status => format!("{status:?} — {reason} ({:?})", d.stage),
    }
}
