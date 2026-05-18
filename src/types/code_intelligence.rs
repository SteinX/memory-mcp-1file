use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeIntelligenceDiagnosticCode {
    Selected,
    Disabled,
    MissingRoot,
    Indexing,
    Ready,
    Degraded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeIntelligenceDiagnostic {
    pub status: CodeIntelligenceDiagnosticCode,
    pub reason_code: CodeIntelligenceDiagnosticCode,
    pub message: String,
}

impl CodeIntelligenceDiagnostic {
    pub fn new(
        status: CodeIntelligenceDiagnosticCode,
        reason_code: CodeIntelligenceDiagnosticCode,
        message: impl Into<String>,
    ) -> Self {
        Self {
            status,
            reason_code,
            message: message.into(),
        }
    }

    pub fn selected(message: impl Into<String>) -> Self {
        Self::new(
            CodeIntelligenceDiagnosticCode::Selected,
            CodeIntelligenceDiagnosticCode::Selected,
            message,
        )
    }

    pub fn disabled(message: impl Into<String>) -> Self {
        Self::new(
            CodeIntelligenceDiagnosticCode::Disabled,
            CodeIntelligenceDiagnosticCode::Disabled,
            message,
        )
    }

    pub fn missing_root(message: impl Into<String>) -> Self {
        Self::new(
            CodeIntelligenceDiagnosticCode::MissingRoot,
            CodeIntelligenceDiagnosticCode::MissingRoot,
            message,
        )
    }

    pub fn indexing(message: impl Into<String>) -> Self {
        Self::new(
            CodeIntelligenceDiagnosticCode::Indexing,
            CodeIntelligenceDiagnosticCode::Indexing,
            message,
        )
    }

    pub fn ready(message: impl Into<String>) -> Self {
        Self::new(
            CodeIntelligenceDiagnosticCode::Ready,
            CodeIntelligenceDiagnosticCode::Ready,
            message,
        )
    }

    pub fn degraded(message: impl Into<String>) -> Self {
        Self::new(
            CodeIntelligenceDiagnosticCode::Degraded,
            CodeIntelligenceDiagnosticCode::Degraded,
            message,
        )
    }

    pub fn as_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_else(|_| serde_json::json!({}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_intelligence_diagnostic_serializes_stable_reason_codes() {
        let cases = [
            (
                CodeIntelligenceDiagnostic::selected("selected"),
                "selected",
                "selected",
            ),
            (
                CodeIntelligenceDiagnostic::disabled("disabled"),
                "disabled",
                "disabled",
            ),
            (
                CodeIntelligenceDiagnostic::missing_root("missing_root"),
                "missing_root",
                "missing_root",
            ),
            (
                CodeIntelligenceDiagnostic::indexing("indexing"),
                "indexing",
                "indexing",
            ),
            (CodeIntelligenceDiagnostic::ready("ready"), "ready", "ready"),
            (
                CodeIntelligenceDiagnostic::degraded("degraded"),
                "degraded",
                "degraded",
            ),
        ];

        for (diagnostic, expected_status, expected_reason_code) in cases {
            let json = serde_json::to_value(&diagnostic).unwrap();
            assert_eq!(json["status"], expected_status);
            assert_eq!(json["reason_code"], expected_reason_code);
            assert!(json["message"].is_string());
        }
    }
}
