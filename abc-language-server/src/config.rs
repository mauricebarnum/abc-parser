// Copyright 2026 Maurice S. Barnum
// SPDX-License-Identifier: Apache-2.0

//! User-configurable validation and formatting preferences.

use serde::Deserialize;

/// Complete configuration for one ABC document.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(default)]
pub struct Config {
    pub(super) validation: ValidationConfig,
    pub(super) format: FormatConfig,
}

/// Parser and advisory settings.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(default)]
pub struct ValidationConfig {
    pub(super) strict: bool,
    #[serde(rename = "ambiguousMusic")]
    pub(super) ambiguous_music: DiagnosticLevel,
    #[serde(rename = "barDuration")]
    pub(super) bar_duration: DiagnosticLevel,
    #[serde(rename = "legacyDecoration")]
    pub(super) legacy_decoration: DiagnosticLevel,
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
    Off,
    Hint,
    Information,
    Warning,
    Error,
}

/// Source-spelling preferences used by formatting and code actions.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(default)]
pub struct FormatConfig {
    #[serde(rename = "noteLength")]
    pub(super) note_length: NoteLengthStyle,
}

/// Preferred spelling for power-of-two note-length divisors.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum NoteLengthStyle {
    #[default]
    Preserve,
    Shorthand,
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
