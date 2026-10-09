// Copyright 2026 Maurice S. Barnum
// SPDX-License-Identifier: Apache-2.0

//! User-configurable validation and formatting preferences.

use serde::Deserialize;

/// Complete configuration for one ABC document.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(default)]
pub struct Config {
    /// Validation and diagnostic settings.
    pub validation: ValidationConfig,
    /// Source formatting preferences.
    pub format: FormatConfig,
}

/// Parser and advisory settings.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(default)]
pub struct ValidationConfig {
    /// Whether strict ABC 2.1 conformance rules are enforced.
    pub strict: bool,
    /// Severity level for missing reference fields on music-like text.
    #[serde(rename = "ambiguousMusic")]
    pub ambiguous_music: DiagnosticLevel,
    /// Severity level for bar-duration mismatch diagnostics.
    #[serde(rename = "barDuration")]
    pub bar_duration: DiagnosticLevel,
    /// Severity level for deprecated legacy decoration warnings.
    #[serde(rename = "legacyDecoration")]
    pub legacy_decoration: DiagnosticLevel,
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self {
            strict: false,
            ambiguous_music: DiagnosticLevel::Warning,
            bar_duration: DiagnosticLevel::Warning,
            legacy_decoration: DiagnosticLevel::Warning,
        }
    }
}

/// Severity selected for an optional diagnostic family.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum DiagnosticLevel {
    /// Suppress the diagnostic entirely.
    Off,
    /// Render the diagnostic as an LSP hint (grey underline).
    Hint,
    /// Render the diagnostic as informational (blue underline).
    Information,
    /// Render the diagnostic as a warning (yellow underline).
    Warning,
    /// Render the diagnostic as an error (red underline).
    Error,
}

/// Source-spelling preferences used by formatting and code actions.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(default)]
pub struct FormatConfig {
    /// Note-length divisor formatting style.
    #[serde(rename = "noteLength")]
    pub note_length: NoteLengthStyle,
}

/// Preferred spelling for power-of-two note-length divisors.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum NoteLengthStyle {
    /// Leave the author's spelling unchanged (default).
    #[default]
    Preserve,
    /// Use repeated slashes (e.g. `A//`).
    Shorthand,
    /// Use explicit numeric denominators (e.g. `A/4`).
    Explicit,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_preserve_source_spelling() {
        let config = Config::default();
        assert_eq!(config.format.note_length, NoteLengthStyle::Preserve);
        assert!(!config.validation.strict);
        assert_eq!(config.validation.ambiguous_music, DiagnosticLevel::Warning);
        assert_eq!(config.validation.bar_duration, DiagnosticLevel::Warning);
    }

    #[test]
    fn deserializes_editor_configuration_names() {
        let config: Config = serde_json::from_value(serde_json::json!({
            "validation": {
                "strict": true,
                "ambiguousMusic": "information",
                "barDuration": "error",
                "legacyDecoration": "off"
            },
            "format": { "noteLength": "explicit" }
        }))
        .expect("configuration should deserialize");
        assert!(config.validation.strict);
        assert_eq!(
            config.validation.ambiguous_music,
            DiagnosticLevel::Information
        );
        assert_eq!(config.validation.bar_duration, DiagnosticLevel::Error);
        assert_eq!(config.validation.legacy_decoration, DiagnosticLevel::Off);
        assert_eq!(config.format.note_length, NoteLengthStyle::Explicit);
    }
}
