//! 버전 타입과 호환성 판정 (V.8.1).
//!
//! 문서(세대)는 자신이 기록된 버전과, 자신을 다루는 데 필요한 최소 버전을
//! 선언합니다:
//!
//! - `format_version`: 컨테이너(chunk/manifest) 포맷 버전. reader가 지원하는
//!   값과 정확히 일치해야 합니다 — 불일치는 거부.
//! - `schema_version`: 문서 스키마 버전(기록 시점).
//! - `minimum_reader_version`: 이 문서를 **읽기**에 필요한 최소 스키마 버전.
//! - `minimum_writer_version`: 이 문서를 **수정 저장**해도 안전한 최소 스키마 버전.
//!
//! reader의 스키마 버전 `A`에 대한 판정:
//!
//! | 조건 | 결과 |
//! |---|---|
//! | `A >= minimum_writer_version` | 편집 가능 |
//! | `minimum_reader_version <= A < minimum_writer_version` | 읽기 전용 |
//! | `A < minimum_reader_version` | 거부 |

use std::fmt;

/// 문서(세대)가 선언하는 버전 집합.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormatVersions {
    /// 컨테이너 포맷 버전.
    pub format_version: u32,
    /// 기록 시점 스키마 버전.
    pub schema_version: u32,
    /// 읽기에 필요한 최소 스키마 버전.
    pub minimum_reader_version: u32,
    /// 수정 저장에 필요한 최소 스키마 버전.
    pub minimum_writer_version: u32,
}

/// 버전 선언 자체의 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionDeclarationError {
    /// minimum_reader_version이 minimum_writer_version보다 큽니다.
    ReaderAboveWriter {
        /// 선언된 reader 최소값.
        reader: u32,
        /// 선언된 writer 최소값.
        writer: u32,
    },
    /// minimum_writer_version이 schema_version보다 큽니다.
    WriterAboveSchema {
        /// 선언된 writer 최소값.
        writer: u32,
        /// 선언된 schema version.
        schema: u32,
    },
}

impl fmt::Display for VersionDeclarationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VersionDeclarationError::ReaderAboveWriter { reader, writer } => {
                write!(
                    f,
                    "minimum_reader_version({reader})이 minimum_writer_version({writer})보다 큽니다"
                )
            }
            VersionDeclarationError::WriterAboveSchema { writer, schema } => {
                write!(
                    f,
                    "minimum_writer_version({writer})이 schema_version({schema})보다 큽니다"
                )
            }
        }
    }
}

impl std::error::Error for VersionDeclarationError {}

impl FormatVersions {
    /// 선언 불변식(`reader <= writer <= schema`)을 검증합니다.
    pub fn validate(&self) -> Result<(), VersionDeclarationError> {
        if self.minimum_reader_version > self.minimum_writer_version {
            return Err(VersionDeclarationError::ReaderAboveWriter {
                reader: self.minimum_reader_version,
                writer: self.minimum_writer_version,
            });
        }
        if self.minimum_writer_version > self.schema_version {
            return Err(VersionDeclarationError::WriterAboveSchema {
                writer: self.minimum_writer_version,
                schema: self.schema_version,
            });
        }
        Ok(())
    }
}

/// 호환성 판정 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Compatibility {
    /// 편집(수정 저장) 가능.
    Editable,
    /// 읽기 전용으로만 열 수 있습니다.
    ReadOnly,
    /// 열 수 없습니다.
    Rejected {
        /// 거부 사유.
        reason: RejectReason,
    },
}

/// 거부 사유.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RejectReason {
    /// 컨테이너 포맷 버전 불일치.
    FormatMismatch {
        /// 문서의 format version.
        document: u32,
        /// reader가 지원하는 format version.
        supported: u32,
    },
    /// reader 스키마 버전이 최소 읽기 버전 미만.
    ReaderTooOld {
        /// reader 스키마 버전.
        reader: u32,
        /// 필요한 최소 버전.
        required: u32,
    },
    /// 버전 선언 자체가 잘못됨.
    InvalidDeclaration(VersionDeclarationError),
}

/// reader(스키마 버전 `reader_schema`, 지원 format `supported_format`)가 문서를
/// 어떻게 열 수 있는지 판정합니다.
pub fn assess_compatibility(
    document: &FormatVersions,
    supported_format: u32,
    reader_schema: u32,
) -> Compatibility {
    if let Err(e) = document.validate() {
        return Compatibility::Rejected {
            reason: RejectReason::InvalidDeclaration(e),
        };
    }
    if document.format_version != supported_format {
        return Compatibility::Rejected {
            reason: RejectReason::FormatMismatch {
                document: document.format_version,
                supported: supported_format,
            },
        };
    }
    if reader_schema < document.minimum_reader_version {
        return Compatibility::Rejected {
            reason: RejectReason::ReaderTooOld {
                reader: reader_schema,
                required: document.minimum_reader_version,
            },
        };
    }
    if reader_schema < document.minimum_writer_version {
        return Compatibility::ReadOnly;
    }
    Compatibility::Editable
}

#[cfg(test)]
mod tests {
    use super::*;

    fn versions(format: u32, schema: u32, reader: u32, writer: u32) -> FormatVersions {
        FormatVersions {
            format_version: format,
            schema_version: schema,
            minimum_reader_version: reader,
            minimum_writer_version: writer,
        }
    }

    #[test]
    fn compatibility_matrix() {
        // 검증 기준(V.8.1): 읽기 전용·편집 가능·거부 matrix test.
        // 문서: format 1, schema 5, 읽기 최소 3, 쓰기 최소 4.
        let doc = versions(1, 5, 3, 4);
        let cases: [(u32, u32, Compatibility); 6] = [
            // (지원 format, reader 스키마, 기대)
            (1, 5, Compatibility::Editable), // 최신 reader
            (1, 4, Compatibility::Editable), // 쓰기 최소 정확히 충족
            (1, 3, Compatibility::ReadOnly), // 읽기만 가능
            (
                1,
                2,
                Compatibility::Rejected {
                    reason: RejectReason::ReaderTooOld {
                        reader: 2,
                        required: 3,
                    },
                },
            ),
            (
                2,
                5,
                Compatibility::Rejected {
                    reason: RejectReason::FormatMismatch {
                        document: 1,
                        supported: 2,
                    },
                },
            ),
            (
                0,
                5,
                Compatibility::Rejected {
                    reason: RejectReason::FormatMismatch {
                        document: 1,
                        supported: 0,
                    },
                },
            ),
        ];
        for (supported_format, reader, expected) in cases {
            assert_eq!(
                assess_compatibility(&doc, supported_format, reader),
                expected,
                "format {supported_format}, reader {reader}"
            );
        }
    }

    #[test]
    fn same_version_everywhere_is_editable() {
        let doc = versions(1, 1, 1, 1);
        assert_eq!(assess_compatibility(&doc, 1, 1), Compatibility::Editable);
    }

    #[test]
    fn newer_reader_than_schema_is_editable() {
        // 신버전 앱이 구버전 문서를 여는 일반 경로.
        let doc = versions(1, 2, 1, 2);
        assert_eq!(assess_compatibility(&doc, 1, 7), Compatibility::Editable);
    }

    #[test]
    fn invalid_declarations_are_rejected() {
        // reader > writer.
        let doc = versions(1, 5, 4, 3);
        assert!(matches!(
            assess_compatibility(&doc, 1, 9),
            Compatibility::Rejected {
                reason: RejectReason::InvalidDeclaration(
                    VersionDeclarationError::ReaderAboveWriter { .. }
                )
            }
        ));
        // writer > schema.
        let doc = versions(1, 3, 2, 4);
        assert!(matches!(
            assess_compatibility(&doc, 1, 9),
            Compatibility::Rejected {
                reason: RejectReason::InvalidDeclaration(
                    VersionDeclarationError::WriterAboveSchema { .. }
                )
            }
        ));
    }
}
