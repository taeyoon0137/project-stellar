//! 단계 trace record schema (XI.2.6, ADR-0084 결정 4).
//!
//! scene·renderer-core는 crate 의존 정책상 diagnostics를 쓰지 않으므로, 각 계층이 정적 event 이름과
//! `(정적 field 이름, u64)` 목록을 내고 diagnostics가 그 목록을 [`TraceRecord`]로 받아 schema로 검사합니다.
//! 값은 모두 정수라 node 이름·text·경로를 담을 수 없습니다.
//!
//! | event | 출처 |
//! |---|---|
//! | `scene.build` | `stellar_scene::incremental::SceneBuildTrace` |
//! | `render_graph.compile` | `stellar_renderer_core::graph_trace::RenderGraphCompileTrace` |

use std::fmt;

/// event 하나의 field schema(순서 고정).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventSchema {
    /// 정적 event 이름.
    pub event: &'static str,
    /// field 이름(순서).
    pub fields: &'static [&'static str],
}

/// scene build(XI.2.6).
pub const SCENE_BUILD: EventSchema = EventSchema {
    event: "scene.build",
    fields: &[
        "scene_revision",
        "start_ns",
        "end_ns",
        "duration_ns",
        "nodes",
        "rebuilt",
        "copied",
        "renumbered",
        "shared",
        "dirty_paint",
        "dirty_geometry",
        "dirty_subtree",
        "full",
    ],
};

/// RenderGraph compile(XI.4.6).
pub const RENDER_GRAPH_COMPILE: EventSchema = EventSchema {
    event: "render_graph.compile",
    fields: &[
        "scene_revision",
        "start_ns",
        "end_ns",
        "cpu_ns",
        "passes",
        "draws",
        "batches",
        "layers",
    ],
};

/// 알려진 schema 전체.
pub const SCHEMAS: [EventSchema; 2] = [SCENE_BUILD, RENDER_GRAPH_COMPILE];

/// 검사할 trace record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceRecord<'a> {
    /// 정적 event 이름.
    pub event: &'static str,
    /// (정적 field 이름, 값).
    pub fields: &'a [(&'static str, u64)],
}

/// schema 위반.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaError {
    /// 모르는 event.
    UnknownEvent {
        /// event.
        event: &'static str,
    },
    /// field 수가 다릅니다.
    FieldCount {
        /// 기대.
        expected: usize,
        /// 실제.
        actual: usize,
    },
    /// field 이름이나 순서가 다릅니다.
    FieldName {
        /// index.
        index: usize,
        /// 기대.
        expected: &'static str,
        /// 실제.
        actual: &'static str,
    },
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SchemaError::UnknownEvent { event } => write!(f, "trace schema에 없는 event {event}"),
            SchemaError::FieldCount { expected, actual } => {
                write!(f, "trace field가 {expected}개여야 하지만 {actual}개입니다")
            }
            SchemaError::FieldName {
                index,
                expected,
                actual,
            } => write!(
                f,
                "trace field {index}는 {expected}여야 하지만 {actual}입니다"
            ),
        }
    }
}

impl std::error::Error for SchemaError {}

/// event 이름의 schema.
pub fn schema(event: &str) -> Option<&'static EventSchema> {
    SCHEMAS.iter().find(|schema| schema.event == event)
}

/// record를 schema로 검사합니다.
pub fn validate(record: &TraceRecord<'_>) -> Result<(), SchemaError> {
    let schema = schema(record.event).ok_or(SchemaError::UnknownEvent {
        event: record.event,
    })?;
    if schema.fields.len() != record.fields.len() {
        return Err(SchemaError::FieldCount {
            expected: schema.fields.len(),
            actual: record.fields.len(),
        });
    }
    for (index, (expected, (actual, _))) in schema.fields.iter().zip(record.fields).enumerate() {
        if expected != actual {
            return Err(SchemaError::FieldName {
                index,
                expected,
                actual,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_build_record_passes_and_drift_fails() {
        // 검증 기준(XI.2.6): trace schema.
        let fields: Vec<(&'static str, u64)> =
            SCENE_BUILD.fields.iter().map(|name| (*name, 1)).collect();
        assert_eq!(
            validate(&TraceRecord {
                event: "scene.build",
                fields: &fields
            }),
            Ok(())
        );
        let mut renamed = fields.clone();
        renamed[5] = ("rebuilt_nodes", 1);
        assert_eq!(
            validate(&TraceRecord {
                event: "scene.build",
                fields: &renamed
            }),
            Err(SchemaError::FieldName {
                index: 5,
                expected: "rebuilt",
                actual: "rebuilt_nodes"
            })
        );
        assert_eq!(
            validate(&TraceRecord {
                event: "scene.build",
                fields: &fields[..3]
            }),
            Err(SchemaError::FieldCount {
                expected: 13,
                actual: 3
            })
        );
        assert_eq!(
            validate(&TraceRecord {
                event: "scene.name",
                fields: &fields
            }),
            Err(SchemaError::UnknownEvent {
                event: "scene.name"
            })
        );
    }

    #[test]
    fn schema_names_are_static_identifiers() {
        for schema in SCHEMAS {
            for name in schema.fields {
                assert!(name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'));
            }
        }
    }
}
