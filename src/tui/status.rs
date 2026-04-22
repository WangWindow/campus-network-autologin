pub(super) struct StatusMessage {
    pub(super) kind: StatusKind,
    pub(super) message: String,
}

impl StatusMessage {
    pub(super) fn info(message: impl Into<String>) -> Self {
        Self {
            kind: StatusKind::Info,
            message: message.into(),
        }
    }

    pub(super) fn success(message: impl Into<String>) -> Self {
        Self {
            kind: StatusKind::Success,
            message: message.into(),
        }
    }

    pub(super) fn error(message: impl Into<String>) -> Self {
        Self {
            kind: StatusKind::Error,
            message: message.into(),
        }
    }
}

pub(super) enum StatusKind {
    Info,
    Success,
    Error,
}
