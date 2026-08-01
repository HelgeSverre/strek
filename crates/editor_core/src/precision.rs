//! Document-owned canvas precision data.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Maximum number of ruler guides accepted in a document.
pub const MAX_GUIDES: usize = 10_000;

/// Stable identifier for a document guide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GuideId(u64);

impl GuideId {
    /// Return the persisted numeric representation.
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Reconstruct a non-zero ID received from persistence or automation.
    pub const fn from_opaque(value: u64) -> Option<Self> {
        if value == 0 {
            None
        } else {
            Some(Self(value))
        }
    }

    pub(crate) const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }
}

/// Axis controlled by a ruler guide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuideAxis {
    Horizontal,
    Vertical,
}

/// One persistent, document-wide ruler guide.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Guide {
    pub id: GuideId,
    pub axis: GuideAxis,
    pub position: f32,
}

/// Configuration for the document's square alignment grid.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GridSettings {
    /// Grid spacing in document units.
    pub spacing: f32,
    /// Number of minor intervals between major lines.
    pub major_every: u8,
}

impl Default for GridSettings {
    fn default() -> Self {
        Self {
            spacing: 10.0,
            major_every: 5,
        }
    }
}

impl GridSettings {
    /// Construct validated grid settings.
    pub fn new(spacing: f32, major_every: u8) -> Result<Self, PrecisionError> {
        let settings = Self {
            spacing,
            major_every,
        };
        settings.validate()?;
        Ok(settings)
    }

    pub(crate) fn validate(self) -> Result<(), PrecisionError> {
        if !self.spacing.is_finite() || self.spacing <= 0.0 {
            return Err(PrecisionError::InvalidGridSpacing);
        }
        if !(1..=10).contains(&self.major_every) {
            return Err(PrecisionError::InvalidMajorInterval);
        }
        Ok(())
    }
}

/// Invalid precision data or operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrecisionError {
    InvalidGridSpacing,
    InvalidMajorInterval,
    InvalidGuideId,
    InvalidGuidePosition,
    DuplicateGuideId(GuideId),
    GuideNotFound(GuideId),
    TooManyGuides,
    IdSpaceExhausted,
}

impl fmt::Display for PrecisionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidGridSpacing => {
                formatter.write_str("grid spacing must be finite and greater than zero")
            }
            Self::InvalidMajorInterval => {
                formatter.write_str("grid major interval must be between 1 and 10")
            }
            Self::InvalidGuideId => formatter.write_str("guide ID must be non-zero"),
            Self::InvalidGuidePosition => formatter.write_str("guide position must be finite"),
            Self::DuplicateGuideId(id) => write!(formatter, "duplicate guide ID {}", id.get()),
            Self::GuideNotFound(id) => write!(formatter, "guide {} does not exist", id.get()),
            Self::TooManyGuides => write!(formatter, "document cannot exceed {MAX_GUIDES} guides"),
            Self::IdSpaceExhausted => formatter.write_str("guide ID space is exhausted"),
        }
    }
}

impl std::error::Error for PrecisionError {}
