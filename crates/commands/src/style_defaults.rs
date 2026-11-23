//! 기본값 세트 선택과 명시 layout longhand command (SD.4, ADR-0082 결정 4·5, ADR-0086 결정 10).
//!
//! - [`SetStyleDefaults`]: 활성 기본값 세트 이름을 바꿉니다. 역은 이전 이름으로 바꾸는 같은 command입니다. 원본
//!   `stellar.yaml` 반영은 저장하는 쪽의 몫이고, 모델은 이름을 거울로 들고 있습니다.
//! - [`UnsetProperty`]: node의 명시 layout longhand 하나를 지웁니다(값 초기화). 그 뒤 값은 세트가 정합니다.
//! - [`SetLayoutProperty`]: 명시 layout longhand 하나를 둡니다. 세트 값과 같아도 명시로 남습니다.
//!
//! 값의 binary 형식은 layout record 형식(`LayoutRecord::encode`, 항목 하나)을 그대로 씁니다.

use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyPath;
use stellar_document::css::style_defaults::{is_builtin, is_valid_set_name};
use stellar_document::layout_record::{
    longhand_violation, LayoutLonghand, LayoutRecord, LayoutValue,
};
use stellar_document::model::{DocumentModel, ModelError};
use stellar_document::modifier_metadata::{InvalidationClass, InvalidationSet};

use crate::codec::{CommandCodecError, Reader, Writer};
use crate::command::{Command, CommandError};
use crate::edit::EditContext;

/// layout longhand 변경의 무효화: layout과 geometry·paint(배치 결과가 바뀜).
const LAYOUT_INVALIDATION: InvalidationSet = InvalidationSet::EMPTY
    .with(InvalidationClass::Layout)
    .with(InvalidationClass::Geometry)
    .with(InvalidationClass::Paint);

fn layout_error(node: StableId, reason: impl Into<String>) -> CommandError {
    CommandError::Model(ModelError::Layout {
        id: node,
        reason: reason.into(),
    })
}

/// `node`가 `longhand`를 명시 값으로 가질 수 있는지 봅니다.
fn check_applies(
    model: &DocumentModel,
    node: StableId,
    longhand: LayoutLonghand,
) -> Result<(), CommandError> {
    let payload = model
        .payload(node)
        .ok_or(CommandError::UnknownNode { id: node })?;
    match longhand_violation(node, longhand, payload) {
        Some(violation) => Err(layout_error(node, violation.to_string())),
        None => Ok(()),
    }
}

fn record_change(ctx: &mut EditContext<'_>, node: StableId, longhand: LayoutLonghand) {
    let key = match ctx.model().payload(node) {
        Some(payload) => longhand.key_for(payload),
        None => longhand.record_key(),
    };
    ctx.changes().property(
        node,
        PropertyPath::with_keys(node, vec![key]),
        LAYOUT_INVALIDATION,
    );
}

fn current_record(model: &DocumentModel, node: StableId) -> LayoutRecord {
    model
        .layout_record(node)
        .cloned()
        .unwrap_or_else(|| LayoutRecord::new(node))
}

fn encode_longhand(out: &mut Writer, longhand: LayoutLonghand) {
    out.u16(longhand.id());
}

fn decode_longhand(input: &mut Reader<'_>) -> Result<LayoutLonghand, CommandCodecError> {
    LayoutLonghand::from_id(input.u16()?).ok_or(CommandCodecError::Malformed {
        reason: "모르는 layout longhand",
    })
}

// --- SetStyleDefaults ---

/// 활성 기본값 세트 이름을 바꿉니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetStyleDefaults {
    /// 새 세트 이름(내장 세트 또는 사용자 세트 이름 형식).
    pub name: String,
}

impl SetStyleDefaults {
    pub(crate) fn validate(&self, _model: &DocumentModel) -> Result<(), CommandError> {
        // 사용자 세트가 실제로 있는지는 프로젝트 파일을 아는 쪽(ops·runtime)이 command를 만들기 전에 봅니다.
        if is_builtin(&self.name) || is_valid_set_name(&self.name) {
            Ok(())
        } else {
            Err(CommandError::InvalidRequest {
                reason: "기본값 세트 이름 형식이 아닙니다",
            })
        }
    }

    pub(crate) fn invert(&self, model: &DocumentModel) -> Result<SetStyleDefaults, CommandError> {
        Ok(SetStyleDefaults {
            name: model.style_defaults().to_string(),
        })
    }

    pub(crate) fn apply(&self, ctx: &mut EditContext<'_>) -> Result<(), CommandError> {
        self.validate(ctx.model())?;
        ctx.set_style_defaults(self.name.clone())?;
        // 세트를 바꾸면 명시하지 않은 모든 node의 배치 입력이 바뀝니다(ADR-0082 Performance).
        let document = ctx.model().document_id();
        ctx.changes()
            .property(document, PropertyPath::new(document), LAYOUT_INVALIDATION);
        Ok(())
    }

    pub(crate) fn encode_into(&self, out: &mut Writer) {
        out.str(&self.name);
    }

    pub(crate) fn decode_from(
        input: &mut Reader<'_>,
    ) -> Result<SetStyleDefaults, CommandCodecError> {
        Ok(SetStyleDefaults {
            name: input.string()?,
        })
    }
}

// --- UnsetProperty ---

/// node의 명시 layout longhand 하나를 지웁니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsetProperty {
    /// 대상 node.
    pub node: StableId,
    /// 지울 longhand.
    pub longhand: LayoutLonghand,
}

impl UnsetProperty {
    pub(crate) fn validate(&self, model: &DocumentModel) -> Result<(), CommandError> {
        check_applies(model, self.node, self.longhand)?;
        if model
            .layout_record(self.node)
            .and_then(|r| r.get(self.longhand))
            .is_none()
        {
            return Err(layout_error(
                self.node,
                format!("{}는 명시 값이 아닙니다", self.longhand),
            ));
        }
        Ok(())
    }

    pub(crate) fn invert(&self, model: &DocumentModel) -> Result<Command, CommandError> {
        self.validate(model)?;
        let value = model
            .layout_record(self.node)
            .and_then(|r| r.get(self.longhand))
            .cloned()
            .expect("validate가 확인");
        Ok(Command::SetLayoutProperty(SetLayoutProperty {
            node: self.node,
            longhand: self.longhand,
            value,
        }))
    }

    pub(crate) fn apply(&self, ctx: &mut EditContext<'_>) -> Result<(), CommandError> {
        self.validate(ctx.model())?;
        let mut record = current_record(ctx.model(), self.node);
        record.unset(self.longhand);
        ctx.set_layout_record(record)?;
        record_change(ctx, self.node, self.longhand);
        Ok(())
    }

    pub(crate) fn encode_into(&self, out: &mut Writer) {
        out.id(self.node);
        encode_longhand(out, self.longhand);
    }

    pub(crate) fn decode_from(input: &mut Reader<'_>) -> Result<UnsetProperty, CommandCodecError> {
        Ok(UnsetProperty {
            node: input.id()?,
            longhand: decode_longhand(input)?,
        })
    }
}

// --- SetLayoutProperty ---

/// node에 명시 layout longhand 하나를 둡니다.
#[derive(Debug, Clone, PartialEq)]
pub struct SetLayoutProperty {
    /// 대상 node.
    pub node: StableId,
    /// longhand.
    pub longhand: LayoutLonghand,
    /// 정규 값.
    pub value: LayoutValue,
}

impl SetLayoutProperty {
    pub(crate) fn validate(&self, model: &DocumentModel) -> Result<(), CommandError> {
        check_applies(model, self.node, self.longhand)?;
        if !self.longhand.accepts(&self.value) {
            return Err(layout_error(
                self.node,
                format!("{}: {}은 정규 값이 아닙니다", self.longhand, self.value),
            ));
        }
        Ok(())
    }

    pub(crate) fn invert(&self, model: &DocumentModel) -> Result<Command, CommandError> {
        self.validate(model)?;
        Ok(
            match model
                .layout_record(self.node)
                .and_then(|r| r.get(self.longhand))
            {
                Some(previous) => Command::SetLayoutProperty(SetLayoutProperty {
                    node: self.node,
                    longhand: self.longhand,
                    value: previous.clone(),
                }),
                None => Command::UnsetProperty(UnsetProperty {
                    node: self.node,
                    longhand: self.longhand,
                }),
            },
        )
    }

    pub(crate) fn apply(&self, ctx: &mut EditContext<'_>) -> Result<(), CommandError> {
        self.validate(ctx.model())?;
        let mut record = current_record(ctx.model(), self.node);
        record
            .set(self.longhand, self.value.clone())
            .map_err(|e| layout_error(self.node, e.to_string()))?;
        ctx.set_layout_record(record)?;
        record_change(ctx, self.node, self.longhand);
        Ok(())
    }

    pub(crate) fn encode_into(&self, out: &mut Writer) {
        let mut record = LayoutRecord::new(self.node);
        record
            .set(self.longhand, self.value.clone())
            .expect("검증한 값");
        out.bytes(&record.encode());
    }

    pub(crate) fn decode_from(
        input: &mut Reader<'_>,
    ) -> Result<SetLayoutProperty, CommandCodecError> {
        let record =
            LayoutRecord::decode(input.bytes()?).map_err(|_| CommandCodecError::Malformed {
                reason: "layout 값 형식",
            })?;
        let mut entries = record.iter();
        let (longhand, value) = entries.next().ok_or(CommandCodecError::Malformed {
            reason: "layout 값이 없습니다",
        })?;
        if entries.next().is_some() {
            return Err(CommandCodecError::Malformed {
                reason: "layout 값은 하나입니다",
            });
        }
        Ok(SetLayoutProperty {
            node: record.node,
            longhand,
            value: value.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{decode_command, encode_command};
    use crate::command::test_fixtures::{sid, small_model as frames};
    use crate::kind::CommandKind;
    use stellar_core_types::order_key::OrderKey;
    use stellar_document::fragment::FragmentPayload;
    use stellar_document::records::{NodePayload, NodeRecord};
    use stellar_document::text::TextPayload;

    /// frame 10(root)·11·12·13에 Text 14와 Fragment 15를 더한 모델.
    fn small_model() -> DocumentModel {
        let mut model = frames();
        for (id, key, payload) in [
            (14, 0x30, NodePayload::Text(TextPayload::empty(0))),
            (15, 0x40, NodePayload::Fragment(FragmentPayload)),
        ] {
            model
                .insert_node(NodeRecord {
                    id: sid(id),
                    parent: Some(sid(10)),
                    order_key: OrderKey::from_bytes(vec![key]).unwrap(),
                    visible: true,
                    locked: false,
                    display_name: String::new(),
                    payload,
                })
                .unwrap();
        }
        model
    }

    fn apply(model: &mut DocumentModel, command: &Command) -> Command {
        command.validate(model).unwrap();
        let inverse = command.invert(model).unwrap();
        let mut ctx = EditContext::new(model);
        command.apply(&mut ctx).unwrap();
        ctx.commit();
        inverse
    }

    fn set(node: u64, longhand: LayoutLonghand, text: &str) -> Command {
        Command::SetLayoutProperty(SetLayoutProperty {
            node: sid(node),
            longhand,
            value: longhand.parse(text).unwrap(),
        })
    }

    #[test]
    fn kinds_numbers_and_codec_round_trip() {
        let commands = [
            Command::SetStyleDefaults(SetStyleDefaults {
                name: "react-native".into(),
            }),
            Command::UnsetProperty(UnsetProperty {
                node: sid(14),
                longhand: LayoutLonghand::FlexGrow,
            }),
            set(
                11,
                LayoutLonghand::GridTemplateColumns,
                "1fr minmax(0px, 2fr)",
            ),
        ];
        let kinds: Vec<u16> = commands.iter().map(|c| c.kind().as_u16()).collect();
        assert_eq!(kinds, [0x0091, 0x0092, 0x0093]);
        for command in &commands {
            assert_eq!(command.kind().codec_version(), 3);
            let bytes = encode_command(command);
            assert_eq!(&decode_command(&bytes).unwrap(), command);
        }
        assert_eq!(CommandKind::SetStyleDefaults.name(), "set-style-defaults");
    }

    #[test]
    fn set_and_unset_round_trip_through_undo() {
        let mut model = small_model();
        let before = model.to_records().unwrap();
        // 명시 값 두기 → 역은 Unset.
        let inverse = apply(&mut model, &set(11, LayoutLonghand::Display, "flex"));
        assert!(matches!(inverse, Command::UnsetProperty(_)));
        assert_eq!(
            model
                .layout_record(sid(11))
                .unwrap()
                .get(LayoutLonghand::Display),
            Some(&LayoutValue::Keyword("flex"))
        );
        // 값 바꾸기 → 역은 이전 값으로 Set.
        let second = apply(&mut model, &set(11, LayoutLonghand::Display, "grid"));
        assert_eq!(second, set(11, LayoutLonghand::Display, "flex"));
        // Unset → 역은 Set(grid).
        let unset = Command::UnsetProperty(UnsetProperty {
            node: sid(11),
            longhand: LayoutLonghand::Display,
        });
        let back = apply(&mut model, &unset);
        assert_eq!(back, set(11, LayoutLonghand::Display, "grid"));
        assert!(model.layout_record(sid(11)).is_none(), "빈 record는 지움");
        // 되돌리기 사슬.
        apply(&mut model, &back);
        apply(&mut model, &second);
        apply(&mut model, &inverse);
        assert_eq!(model.to_records().unwrap(), before);
        // 명시 값이 없는 longhand의 Unset은 거부.
        assert!(unset.validate(&model).is_err());
    }

    #[test]
    fn container_longhands_on_non_frames_and_bad_values_are_rejected() {
        let model = small_model();
        let rejected = [
            // Fragment에는 record가 없습니다.
            set(15, LayoutLonghand::Left, "1px"),
            // Frame payload 크기는 payload가 저장 위치.
            set(11, LayoutLonghand::Width, "auto"),
            // Text에는 container longhand가 없습니다.
            set(14, LayoutLonghand::Display, "flex"),
            Command::SetLayoutProperty(SetLayoutProperty {
                node: sid(11),
                longhand: LayoutLonghand::Display,
                value: LayoutValue::Keyword("inline"),
            }),
            set(99, LayoutLonghand::Left, "1px"),
        ];
        for command in rejected {
            assert!(command.validate(&model).is_err(), "{command:?}");
        }
        // Text의 item longhand(width 포함)는 받습니다.
        set(14, LayoutLonghand::Width, "fit-content")
            .validate(&model)
            .unwrap();
        assert!(Command::SetStyleDefaults(SetStyleDefaults {
            name: "Bad Name".into()
        })
        .validate(&model)
        .is_err());
    }

    #[test]
    fn style_defaults_selection_undoes_in_one_step() {
        let mut model = small_model();
        assert_eq!(model.style_defaults(), "css");
        let command = Command::SetStyleDefaults(SetStyleDefaults {
            name: "react-native".into(),
        });
        let inverse = apply(&mut model, &command);
        assert_eq!(model.style_defaults(), "react-native");
        apply(&mut model, &inverse);
        assert_eq!(model.style_defaults(), "css");
    }

    #[test]
    fn failed_transaction_rolls_back_layout_and_selection() {
        let mut model = small_model();
        let mut ctx = EditContext::new(&mut model);
        set(11, LayoutLonghand::Display, "flex")
            .apply(&mut ctx)
            .unwrap();
        Command::SetStyleDefaults(SetStyleDefaults {
            name: "react-native".into(),
        })
        .apply(&mut ctx)
        .unwrap();
        ctx.rollback().unwrap();
        drop(ctx);
        assert!(model.layout_record(sid(11)).is_none());
        assert_eq!(model.style_defaults(), "css");
    }
}
