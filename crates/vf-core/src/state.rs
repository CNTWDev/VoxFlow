use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state")]
pub enum RecorderState {
    Idle,
    Recording,
    Processing,
    Injecting,
    Error { message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variants_are_distinct() {
        assert_ne!(RecorderState::Idle, RecorderState::Recording);
        assert_ne!(RecorderState::Recording, RecorderState::Processing);
        assert_ne!(RecorderState::Processing, RecorderState::Injecting);
    }

    #[test]
    fn error_variant_equality() {
        let a = RecorderState::Error { message: "boom".into() };
        let b = RecorderState::Error { message: "boom".into() };
        let c = RecorderState::Error { message: "other".into() };
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn clone_preserves_value() {
        let s = RecorderState::Error { message: "x".into() };
        assert_eq!(s.clone(), s);
    }

    #[test]
    fn idle_matches_pattern() {
        let s = RecorderState::Idle;
        assert!(matches!(s, RecorderState::Idle));
        assert!(!matches!(s, RecorderState::Recording));
    }
}
