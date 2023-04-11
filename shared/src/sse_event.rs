use std::{convert::Infallible, fmt::Display, str::FromStr};

pub enum SseEventType {
    NewTask,
    NewResult,
    UpdatedTask,
    UpdatedResult,
    WaitExpired,
    DeletedTask,
    Error,
    Undefined,
    Unknown(String),
}

impl AsRef<str> for SseEventType {
    fn as_ref(&self) -> &str {
        match self {
            SseEventType::NewTask => "new_task",
            SseEventType::NewResult => "new_result",
            SseEventType::UpdatedTask => "updated_task",
            SseEventType::UpdatedResult => "updated_result",
            SseEventType::WaitExpired => "wait_expired",
            SseEventType::DeletedTask => "deleted_task",
            SseEventType::Error => "error",
            SseEventType::Undefined => "", // Make this "message"?
            SseEventType::Unknown(e) => e.as_str(),
        }
    }
}

impl Display for SseEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_ref())
    }
}

impl FromStr for SseEventType {
    type Err = Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "new_task" => Self::NewTask,
            "new_result" => Self::NewResult,
            "updated_task" => Self::UpdatedTask,
            "updated_result" => Self::UpdatedResult,
            "wait_expired" => Self::WaitExpired,
            "deleted_task" => Self::DeletedTask,
            "error" => Self::Error,
            "message" => Self::Undefined,
            unknown => Self::Unknown(unknown.to_string()),
        })
    }
}
