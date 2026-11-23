//! 형식 중립 command 입력 해석과 command 종류 표 (XIX.5.1, XIX.9.2, ADR-0085 결정 2·3).
//!
//! `stellar-apply` command object(`kind` + 필드, 또는 `kind` + `canonical`)를 [`CommandInput`]으로 바꿉니다.
//! JSON 구현과 무관하게 [`CommandFields`]로 값을 읽으므로 L2 MCP와 L1 도구가 같은 해석 규칙을 씁니다.
//! 바뀐 결과는 [`crate::apply`]가 commands crate의 **닫힌 enum**([`stellar_commands::command::Command`])으로만
//! 바꿉니다. 등록되지 않은 종류는 `unknown-command`로 거부하며 raw patch·script 형식은 없습니다.
//!
//! - [`FIELD_SPECS`]: 필드 형식 종류와 각 필드의 값 형식. xtask가 이 표와 [`canonical_kinds`]로
//!   `schemas/command-transaction.schema.json`을 생성합니다(`cargo xtask generate-command-schema`).
//! - 해석 규칙은 `stellar apply`(XXIV.4.4) 입력 형식 표와 같습니다. 숫자는 JSON 원문에서 바로 f32로 바꿔
//!   f64를 거치는 이중 반올림을 피합니다.
//! - Component Context 편집(`edit-context`, ADR-0099 결정 8)은 연산마다 필드 종류 하나(`create-context-key` …)로
//!   받습니다([`CONTEXT_FIELD_KINDS`], ADR-0114 결정 3). 결과는 같은 `edit-context` command의 canonical 입력이라 적용
//!   경로는 다른 closed enum 종류와 하나입니다. scope·owner·값 타입 문장과 리터럴은 텍스트 원본
//!   (`design/context.stellar.yaml`)과 같은 함수로 읽습니다. 식 필드(`default`·`value`)는 다음 셋 중 하나입니다.
//!
//!   | 형식 | 뜻 |
//!   |---|---|
//!   | `<타입> <리터럴>` (예: `number 8`, `color #1e90ffff`, `length 12px`) | 리터럴 하나. root id는 key·provision StableId |
//!   | `context <key StableId>` | 그 key 읽기 하나(provision 값 식 전용, key default는 닫힌 식이어야 함) |
//!   | `ast <hex>` | canonical AST bytes |
//!
//! 이 모듈은 AI·모델·에이전트 코드가 아닙니다. 파일을 읽거나 쓰지 않습니다.

use std::fmt;

use stellar_commands::codec::encode_command;
use stellar_commands::command::Command;
use stellar_commands::context::{ContextCommand, ContextEdit};
use stellar_commands::kind::CommandKind;
use stellar_document::context::key::ContextKey;
use stellar_document::context::provision::ContextProvision;
use stellar_document::expressions::ast::{ExprNode, ExprNodeId, ExpressionAst, ReferenceTarget};
use stellar_project::document_io::source_v4::context::{
    ast_expression, literal_expression, parse_owner, parse_scope, parse_type,
};
use stellar_storage::hash::ContentHash;

use crate::apply::{CommandInput, ComponentInstanceInsert, ReplaceText, FIELD_KINDS};
use crate::core_types::ids::StableId;
use crate::core_types::value::{FiniteF32, Size2, Transform2D, Vec2};

/// 필드 값 형식.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldType {
    /// StableId 문자열.
    NodeId,
    /// JSON bool.
    Bool,
    /// JSON 문자열.
    Text,
    /// 음이 아닌 u32 정수.
    U32,
    /// 유한 f32 범위의 JSON 숫자.
    F32,
    /// 음이 아닌 유한 f32 범위의 JSON 숫자.
    NonNegativeF32,
    /// 짝수 길이 소문자 16진수 bytes.
    Hex,
    /// `sha256:<64자리 소문자 16진수>` 내용 hash.
    ContentHash,
}

impl FieldType {
    /// schema·문서용 고정 이름.
    pub fn name(self) -> &'static str {
        match self {
            FieldType::NodeId => "stable-id",
            FieldType::Bool => "bool",
            FieldType::Text => "text",
            FieldType::U32 => "u32",
            FieldType::F32 => "f32",
            FieldType::NonNegativeF32 => "non-negative-f32",
            FieldType::Hex => "hex",
            FieldType::ContentHash => "content-hash",
        }
    }
}

/// 필드 형식 종류 하나.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldSpec {
    /// 종류 이름(`kind`).
    pub kind: &'static str,
    /// `kind` 밖의 필수 필드(이 순서로 문서화).
    pub fields: &'static [(&'static str, FieldType)],
}

/// canonical 형식 command의 bytes 필드 이름.
pub const CANONICAL_FIELD: &str = "canonical";

/// 필드 형식 종류([`FIELD_KINDS`]와 같은 순서).
pub const FIELD_SPECS: &[FieldSpec] = &[
    FieldSpec {
        kind: "set-visible",
        fields: &[("node", FieldType::NodeId), ("visible", FieldType::Bool)],
    },
    FieldSpec {
        kind: "set-locked",
        fields: &[("node", FieldType::NodeId), ("locked", FieldType::Bool)],
    },
    FieldSpec {
        kind: "rename",
        fields: &[("node", FieldType::NodeId), ("name", FieldType::Text)],
    },
    FieldSpec {
        kind: "replace-text",
        fields: &[
            ("node", FieldType::NodeId),
            ("start", FieldType::U32),
            ("end", FieldType::U32),
            ("text", FieldType::Text),
        ],
    },
    FieldSpec {
        kind: "set-frame-size",
        fields: &[
            ("node", FieldType::NodeId),
            ("width", FieldType::NonNegativeF32),
            ("height", FieldType::NonNegativeF32),
        ],
    },
    FieldSpec {
        kind: "set-frame-width",
        fields: &[
            ("node", FieldType::NodeId),
            ("width", FieldType::NonNegativeF32),
        ],
    },
    FieldSpec {
        kind: "set-frame-height",
        fields: &[
            ("node", FieldType::NodeId),
            ("height", FieldType::NonNegativeF32),
        ],
    },
    FieldSpec {
        kind: "set-frame-transform",
        fields: &[
            ("node", FieldType::NodeId),
            ("a", FieldType::F32),
            ("b", FieldType::F32),
            ("c", FieldType::F32),
            ("d", FieldType::F32),
            ("tx", FieldType::F32),
            ("ty", FieldType::F32),
        ],
    },
    FieldSpec {
        kind: "delete-subtree",
        fields: &[("root", FieldType::NodeId)],
    },
    FieldSpec {
        kind: "move-node",
        fields: &[
            ("node", FieldType::NodeId),
            ("new_parent", FieldType::NodeId),
            ("order_key", FieldType::Hex),
        ],
    },
    FieldSpec {
        kind: "insert-component-instance",
        fields: &[
            ("parent", FieldType::NodeId),
            ("component", FieldType::NodeId),
            ("source_hash", FieldType::ContentHash),
            ("x", FieldType::F32),
            ("y", FieldType::F32),
        ],
    },
    FieldSpec {
        kind: "set-property",
        fields: &[
            ("node", FieldType::NodeId),
            ("property", FieldType::Text),
            ("value", FieldType::Text),
        ],
    },
    FieldSpec {
        kind: "set-layout-property",
        fields: &[
            ("node", FieldType::NodeId),
            ("property", FieldType::Text),
            ("value", FieldType::Text),
        ],
    },
    FieldSpec {
        kind: "unset-property",
        fields: &[("node", FieldType::NodeId), ("property", FieldType::Text)],
    },
    FieldSpec {
        kind: "create-context-key",
        fields: &[
            ("key", FieldType::NodeId),
            ("name", FieldType::Text),
            ("scope", FieldType::Text),
            ("value_type", FieldType::Text),
            ("default", FieldType::Text),
        ],
    },
    FieldSpec {
        kind: "rename-context-key",
        fields: &[("key", FieldType::NodeId), ("name", FieldType::Text)],
    },
    FieldSpec {
        kind: "set-context-key-default",
        fields: &[
            ("key", FieldType::NodeId),
            ("value_type", FieldType::Text),
            ("default", FieldType::Text),
        ],
    },
    FieldSpec {
        kind: "delete-context-key",
        fields: &[("key", FieldType::NodeId)],
    },
    FieldSpec {
        kind: "create-context-provision",
        fields: &[
            ("provision", FieldType::NodeId),
            ("owner", FieldType::Text),
            ("provider", FieldType::NodeId),
            ("key", FieldType::NodeId),
            ("value", FieldType::Text),
        ],
    },
    FieldSpec {
        kind: "set-context-provision-value",
        fields: &[("provision", FieldType::NodeId), ("value", FieldType::Text)],
    },
    FieldSpec {
        kind: "delete-context-provision",
        fields: &[("provision", FieldType::NodeId)],
    },
    FieldSpec {
        kind: "add-context-requirement",
        fields: &[("component", FieldType::NodeId), ("key", FieldType::NodeId)],
    },
    FieldSpec {
        kind: "remove-context-requirement",
        fields: &[("component", FieldType::NodeId), ("key", FieldType::NodeId)],
    },
    FieldSpec {
        kind: "add-slot-context-key",
        fields: &[("slot", FieldType::NodeId), ("key", FieldType::NodeId)],
    },
    FieldSpec {
        kind: "remove-slot-context-key",
        fields: &[("slot", FieldType::NodeId), ("key", FieldType::NodeId)],
    },
];

/// Context 편집 필드 종류([`FIELD_SPECS`] 끝 11개). 모두 `edit-context` command 하나가 됩니다.
pub const CONTEXT_FIELD_KINDS: [&str; 11] = [
    "create-context-key",
    "rename-context-key",
    "set-context-key-default",
    "delete-context-key",
    "create-context-provision",
    "set-context-provision-value",
    "delete-context-provision",
    "add-context-requirement",
    "remove-context-requirement",
    "add-slot-context-key",
    "remove-slot-context-key",
];

/// canonical 형식(`kind` + [`CANONICAL_FIELD`])으로만 받는 closed enum 종류 이름(번호 오름차순). 필드 형식이
/// 있는 종류는 필드로만 받습니다.
pub fn canonical_kinds() -> impl Iterator<Item = &'static str> {
    CommandKind::ALL
        .iter()
        .map(|kind| kind.name())
        .filter(|name| !FIELD_KINDS.contains(name))
}

/// 한 command object의 값을 읽는 형식 중립 접근자. JSON object 하나에 대응합니다.
pub trait CommandFields {
    /// object의 key 목록. object가 아니면 `None`.
    fn keys(&self) -> Option<Vec<&str>>;
    /// 문자열 값.
    fn text(&self, key: &str) -> Option<&str>;
    /// bool 값.
    fn boolean(&self, key: &str) -> Option<bool>;
    /// 숫자 값의 원문(JSON number 표기).
    fn number_text(&self, key: &str) -> Option<&str>;
    /// 정수 값.
    fn integer(&self, key: &str) -> Option<i64>;
    /// key가 있는지.
    fn has(&self, key: &str) -> bool;
}

/// 해석 실패. `code`는 `stellar apply` 진단 코드와 같습니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandFieldError {
    /// `unknown-command` 또는 `invalid-transaction`.
    pub code: &'static str,
    /// 사람이 읽는 원인(요청 위치 포함, 프로젝트 내용 없음).
    pub message: String,
}

impl CommandFieldError {
    fn invalid(message: impl Into<String>) -> Self {
        CommandFieldError {
            code: "invalid-transaction",
            message: message.into(),
        }
    }

    /// closed enum에 없는 종류인지.
    pub fn is_unknown_command(&self) -> bool {
        self.code == "unknown-command"
    }
}

impl fmt::Display for CommandFieldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CommandFieldError {}

struct Reader<'a> {
    value: &'a dyn CommandFields,
    at: &'a str,
}

impl Reader<'_> {
    fn only_keys(&self, allowed: &[&str]) -> Result<(), CommandFieldError> {
        let keys = self.value.keys().ok_or_else(|| {
            CommandFieldError::invalid(format!("{}는 object여야 합니다", self.at))
        })?;
        if let Some(key) = keys.iter().find(|key| !allowed.contains(key)) {
            return Err(CommandFieldError::invalid(format!(
                "{}에 허용되지 않은 key {key:?}가 있습니다",
                self.at
            )));
        }
        Ok(())
    }

    fn require(&self, key: &str) -> Result<(), CommandFieldError> {
        if self.value.has(key) {
            Ok(())
        } else {
            Err(CommandFieldError::invalid(format!(
                "{}에 {key}가 없습니다",
                self.at
            )))
        }
    }

    fn text(&self, key: &str) -> Result<&str, CommandFieldError> {
        self.require(key)?;
        self.value.text(key).ok_or_else(|| {
            CommandFieldError::invalid(format!("{}.{key}는 문자열이어야 합니다", self.at))
        })
    }

    fn id(&self, key: &str) -> Result<StableId, CommandFieldError> {
        self.text(key)?.parse::<StableId>().map_err(|_| {
            CommandFieldError::invalid(format!("{}.{key}는 StableId여야 합니다", self.at))
        })
    }

    fn boolean(&self, key: &str) -> Result<bool, CommandFieldError> {
        self.require(key)?;
        self.value.boolean(key).ok_or_else(|| {
            CommandFieldError::invalid(format!("{}.{key}는 bool이어야 합니다", self.at))
        })
    }

    fn f32(&self, key: &str) -> Result<FiniteF32, CommandFieldError> {
        self.require(key)?;
        let bad = || {
            CommandFieldError::invalid(format!(
                "{}.{key}는 유한 f32 범위의 JSON 숫자여야 합니다",
                self.at
            ))
        };
        let number = self
            .value
            .number_text(key)
            .ok_or_else(bad)?
            .parse::<f32>()
            .map_err(|_| bad())?;
        FiniteF32::new(number).map_err(|_| bad())
    }

    fn u32(&self, key: &str) -> Result<u32, CommandFieldError> {
        self.require(key)?;
        self.value
            .integer(key)
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| {
                CommandFieldError::invalid(format!(
                    "{}.{key}는 음이 아닌 u32 정수여야 합니다",
                    self.at
                ))
            })
    }

    /// Context 식 필드(모듈 문서 표). 리터럴과 key 읽기의 root id는 `owner`입니다.
    fn expression(&self, key: &str, owner: StableId) -> Result<ExpressionAst, CommandFieldError> {
        let text = self.text(key)?;
        let bad = |why: String| CommandFieldError::invalid(format!("{}.{key}: {why}", self.at));
        let (head, rest) = text.split_once(' ').ok_or_else(|| {
            bad("`<타입> <리터럴>`, `context <key>`, `ast <hex>` 중 하나여야 합니다".into())
        })?;
        match head {
            "ast" => ast_expression(rest).map_err(|e| bad(e.to_string())),
            "context" => {
                let read = rest
                    .parse::<StableId>()
                    .map_err(|_| bad("context 뒤에는 key StableId가 와야 합니다".into()))?;
                let root = ExprNodeId::new(owner);
                ExpressionAst::new(
                    root,
                    vec![ExprNode::reference(root, ReferenceTarget::Context(read))],
                )
                .map_err(|e| bad(e.to_string()))
            }
            kind => literal_expression(owner, kind, rest).map_err(|e| bad(e.to_string())),
        }
    }

    fn hex(&self, key: &str) -> Result<Vec<u8>, CommandFieldError> {
        let text = self.text(key)?;
        let bad =
            || CommandFieldError::invalid(format!("{}.{key}는 소문자 16진수여야 합니다", self.at));
        if text.len() % 2 != 0
            || !text
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(bad());
        }
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).map_err(|_| bad()))
            .collect()
    }
}

fn allowed_keys(spec: &FieldSpec) -> Vec<&'static str> {
    std::iter::once("kind")
        .chain(spec.fields.iter().map(|(name, _)| *name))
        .collect()
}

/// command object 하나를 [`CommandInput`]으로 해석합니다. `at`은 오류 문구의 위치(예: `commands[3]`)입니다.
///
/// 필드 형식 종류는 [`FIELD_SPECS`]의 key만, 그 밖의 종류는 `kind`·`canonical`만 가질 수 있습니다.
/// `canonical`이 없는 모르는 종류는 `unknown-command`입니다. canonical bytes가 선언한 종류와 다르거나
/// closed enum에 없으면 [`crate::apply`]가 적용 전에 거부합니다.
pub fn command_input(
    value: &dyn CommandFields,
    at: &str,
) -> Result<CommandInput, CommandFieldError> {
    let r = Reader { value, at };
    if value.keys().is_none() {
        return Err(CommandFieldError::invalid(format!(
            "{at}는 object여야 합니다"
        )));
    }
    let kind = r.text("kind")?;
    let Some(spec) = FIELD_SPECS.iter().find(|spec| spec.kind == kind) else {
        if !value.has(CANONICAL_FIELD) {
            return Err(CommandFieldError {
                code: "unknown-command",
                message: format!("{at}: 알 수 없는 command 종류 {kind:?}"),
            });
        }
        r.only_keys(&["kind", CANONICAL_FIELD])?;
        return Ok(CommandInput::Canonical {
            kind: kind.to_string(),
            bytes: r.hex(CANONICAL_FIELD)?,
        });
    };
    r.only_keys(&allowed_keys(spec))?;
    let input = match kind {
        "set-visible" => CommandInput::SetVisible {
            node: r.id("node")?,
            visible: r.boolean("visible")?,
        },
        "set-locked" => CommandInput::SetLocked {
            node: r.id("node")?,
            locked: r.boolean("locked")?,
        },
        "rename" => CommandInput::Rename {
            node: r.id("node")?,
            name: r.text("name")?.to_string(),
        },
        "replace-text" => CommandInput::ReplaceText(
            ReplaceText::new(
                r.id("node")?,
                r.u32("start")?,
                r.u32("end")?,
                r.text("text")?.into(),
            )
            .ok_or_else(|| CommandFieldError::invalid("Text 범위의 시작은 끝 이하여야 합니다"))?,
        ),
        "set-frame-size" => {
            let size = Size2::new(r.f32("width")?.get(), r.f32("height")?.get())
                .map_err(|error| CommandFieldError::invalid(format!("{at}: {error}")))?;
            CommandInput::SetFrameSize {
                node: r.id("node")?,
                size,
            }
        }
        "set-frame-width" | "set-frame-height" => {
            let field = if kind == "set-frame-width" {
                "width"
            } else {
                "height"
            };
            let dimension = r.f32(field)?;
            if dimension.get() < 0.0 {
                return Err(CommandFieldError::invalid(format!(
                    "{at}.{field}는 음수일 수 없습니다"
                )));
            }
            let node = r.id("node")?;
            if field == "width" {
                CommandInput::SetFrameWidth {
                    node,
                    width: dimension,
                }
            } else {
                CommandInput::SetFrameHeight {
                    node,
                    height: dimension,
                }
            }
        }
        "set-frame-transform" => {
            let transform = Transform2D::new(
                r.f32("a")?.get(),
                r.f32("b")?.get(),
                r.f32("c")?.get(),
                r.f32("d")?.get(),
                r.f32("tx")?.get(),
                r.f32("ty")?.get(),
            )
            .map_err(|error| CommandFieldError::invalid(format!("{at}: {error}")))?;
            CommandInput::SetFrameTransform {
                node: r.id("node")?,
                transform,
            }
        }
        "delete-subtree" => CommandInput::DeleteSubtree {
            root: r.id("root")?,
        },
        "move-node" => CommandInput::MoveNode {
            node: r.id("node")?,
            new_parent: r.id("new_parent")?,
            order_key: r.hex("order_key")?,
        },
        "insert-component-instance" => {
            let source_hash = r.text("source_hash")?.parse::<ContentHash>().map_err(|_| {
                CommandFieldError::invalid(format!(
                    "{at}.source_hash는 sha256:<64자리 소문자 16진수>여야 합니다"
                ))
            })?;
            let position = Vec2::new(r.f32("x")?.get(), r.f32("y")?.get())
                .map_err(|_| CommandFieldError::invalid(format!("{at}: 위치는 유한해야 합니다")))?;
            CommandInput::InsertComponentInstance(ComponentInstanceInsert {
                component: r.id("component")?,
                source_hash,
                parent: r.id("parent")?,
                position,
            })
        }
        "set-property" => CommandInput::SetProperty {
            node: r.id("node")?,
            property: r.text("property")?.to_string(),
            value: r.text("value")?.to_string(),
        },
        "set-layout-property" => CommandInput::SetLayoutProperty {
            node: r.id("node")?,
            property: r.text("property")?.to_string(),
            value: r.text("value")?.to_string(),
        },
        "unset-property" => CommandInput::UnsetProperty {
            node: r.id("node")?,
            property: r.text("property")?.to_string(),
        },
        kind if CONTEXT_FIELD_KINDS.contains(&kind) => context_input(&r, kind)?,
        _ => unreachable!("FIELD_SPECS의 모든 종류를 위에서 다룹니다"),
    };
    Ok(input)
}

/// Context 편집 필드 종류를 `edit-context` canonical 입력으로 바꿉니다(ADR-0114 결정 3).
fn context_input(r: &Reader<'_>, kind: &str) -> Result<CommandInput, CommandFieldError> {
    let at = r.at;
    let source =
        |what: &str, e: &dyn fmt::Display| CommandFieldError::invalid(format!("{at}.{what}: {e}"));
    let edit = match kind {
        "create-context-key" => {
            let key = r.id("key")?;
            let value_type =
                parse_type(r.text("value_type")?).map_err(|e| source("value_type", &e))?;
            let scope = parse_scope(r.text("scope")?).map_err(|e| source("scope", &e))?;
            ContextEdit::CreateKey(
                ContextKey::new(
                    key,
                    r.text("name")?,
                    scope,
                    value_type,
                    r.expression("default", key)?,
                )
                .map_err(|e| source("default", &e))?,
            )
        }
        "rename-context-key" => ContextEdit::RenameKey {
            key: r.id("key")?,
            name: r.text("name")?.to_owned(),
        },
        "set-context-key-default" => {
            let key = r.id("key")?;
            ContextEdit::SetKeyDefault {
                key,
                value_type: parse_type(r.text("value_type")?)
                    .map_err(|e| source("value_type", &e))?,
                default: r.expression("default", key)?,
            }
        }
        "delete-context-key" => ContextEdit::DeleteKey { key: r.id("key")? },
        "create-context-provision" => {
            let provision = r.id("provision")?;
            let owner = parse_owner(r.text("owner")?).map_err(|e| source("owner", &e))?;
            ContextEdit::CreateProvision(
                ContextProvision::new(
                    provision,
                    owner,
                    r.id("provider")?,
                    r.id("key")?,
                    r.expression("value", provision)?,
                )
                .map_err(|e| source("value", &e))?,
            )
        }
        "set-context-provision-value" => {
            let provision = r.id("provision")?;
            ContextEdit::SetProvisionValue {
                provision,
                value: r.expression("value", provision)?,
            }
        }
        "delete-context-provision" => ContextEdit::DeleteProvision {
            provision: r.id("provision")?,
        },
        "add-context-requirement" => ContextEdit::AddRequirement {
            component: r.id("component")?,
            key: r.id("key")?,
        },
        "remove-context-requirement" => ContextEdit::RemoveRequirement {
            component: r.id("component")?,
            key: r.id("key")?,
        },
        "add-slot-context-key" => ContextEdit::AddSlotKey {
            slot: r.id("slot")?,
            key: r.id("key")?,
        },
        "remove-slot-context-key" => ContextEdit::RemoveSlotKey {
            slot: r.id("slot")?,
            key: r.id("key")?,
        },
        _ => unreachable!("CONTEXT_FIELD_KINDS의 모든 종류를 위에서 다룹니다"),
    };
    let command = Command::EditContext(Box::new(ContextCommand { edit }));
    Ok(CommandInput::Canonical {
        kind: command.kind().name().to_owned(),
        bytes: encode_command(&command),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// 테스트용 값: 문자열·bool·숫자 원문.
    #[derive(Clone)]
    enum V {
        S(&'static str),
        B(bool),
        N(&'static str),
    }

    struct Obj(BTreeMap<&'static str, V>);

    impl CommandFields for Obj {
        fn keys(&self) -> Option<Vec<&str>> {
            Some(self.0.keys().copied().collect())
        }
        fn text(&self, key: &str) -> Option<&str> {
            match self.0.get(key) {
                Some(V::S(s)) => Some(s),
                _ => None,
            }
        }
        fn boolean(&self, key: &str) -> Option<bool> {
            match self.0.get(key) {
                Some(V::B(b)) => Some(*b),
                _ => None,
            }
        }
        fn number_text(&self, key: &str) -> Option<&str> {
            match self.0.get(key) {
                Some(V::N(n)) => Some(n),
                _ => None,
            }
        }
        fn integer(&self, key: &str) -> Option<i64> {
            self.number_text(key).and_then(|n| n.parse().ok())
        }
        fn has(&self, key: &str) -> bool {
            self.0.contains_key(key)
        }
    }

    fn obj(pairs: &[(&'static str, V)]) -> Obj {
        Obj(pairs.iter().cloned().collect())
    }

    const NODE: &str = "0195c530-3acd-7bc1-8414-c213f61cb001";

    #[test]
    fn field_specs_cover_exactly_the_field_kinds() {
        let kinds: Vec<&str> = FIELD_SPECS.iter().map(|s| s.kind).collect();
        assert_eq!(kinds, FIELD_KINDS);
        // canonical 종류는 closed enum에서 필드 종류를 뺀 나머지이고 겹치지 않습니다.
        let canonical: Vec<&str> = canonical_kinds().collect();
        assert!(!canonical.is_empty());
        assert!(canonical.iter().all(|k| !FIELD_KINDS.contains(k)));
        let all: Vec<&str> = CommandKind::ALL.iter().map(|k| k.name()).collect();
        assert!(canonical.iter().all(|k| all.contains(k)));
        assert!(canonical.contains(&"insert-node") && !canonical.contains(&"move-node"));
    }

    #[test]
    fn unknown_command_without_canonical_is_rejected() {
        // 검증 기준(XIX.5.1): unknown command 거부. raw patch·script 형식은 없습니다.
        for kind in ["raw-patch", "script", "eval", "memory-write"] {
            let error = command_input(&obj(&[("kind", V::S(kind))]), "commands[0]").unwrap_err();
            assert!(error.is_unknown_command(), "{kind}");
        }
        // canonical을 붙여도 closed enum에 없는 종류는 적용 전 변환에서 거부됩니다.
        let input = command_input(
            &obj(&[("kind", V::S("raw-patch")), ("canonical", V::S("00"))]),
            "commands[0]",
        )
        .unwrap();
        assert!(matches!(
            input.to_command(),
            Err(crate::apply::CommandInputError::UnknownKind(_))
        ));
    }

    fn context_edit(input: CommandInput) -> ContextEdit {
        match input.to_command().unwrap() {
            Command::EditContext(command) => command.edit,
            other => panic!("edit-context가 아님: {other:?}"),
        }
    }

    const KEY: &str = "0195c530-3acd-7bc1-8414-c213f61cb101";
    const PROVISION: &str = "0195c530-3acd-7bc1-8414-c213f61cb102";
    const VIEW: &str = "0195c530-3acd-7bc1-8414-c213f61cb103";
    const OTHER: &str = "0195c530-3acd-7bc1-8414-c213f61cb104";

    #[test]
    fn context_field_kinds_become_one_edit_context_command() {
        // 검증 기준(XXX.7.8 남은 범위, ADR-0114 결정 3): CLI·MCP 공용 해석이 Context 편집을 받습니다.
        let create = command_input(
            &obj(&[
                ("kind", V::S("create-context-key")),
                ("key", V::S(KEY)),
                ("name", V::S("spacing")),
                ("scope", V::S("project")),
                ("value_type", V::S("length")),
                ("default", V::S("length 8px")),
            ]),
            "commands[0]",
        )
        .unwrap();
        assert!(matches!(&create, CommandInput::Canonical { kind, .. } if kind == "edit-context"));
        let ContextEdit::CreateKey(key) = context_edit(create) else {
            panic!("key 생성");
        };
        assert_eq!(key.id(), KEY.parse().unwrap());
        assert_eq!(key.name(), "spacing");
        assert_eq!(
            key.default_expression().ast().root().stable_id(),
            KEY.parse().unwrap(),
            "리터럴 root id는 key id"
        );
        // provision 값은 다른 key 읽기(`context <key>`)도 받습니다.
        let provision = command_input(
            &obj(&[
                ("kind", V::S("create-context-provision")),
                ("provision", V::S(PROVISION)),
                ("owner", V::S(format_view())),
                ("provider", V::S(NODE)),
                ("key", V::S(KEY)),
                ("value", V::S(format_read())),
            ]),
            "commands[0]",
        )
        .unwrap();
        let ContextEdit::CreateProvision(provision) = context_edit(provision) else {
            panic!("provision 생성");
        };
        assert_eq!(
            provision.reads().into_iter().collect::<Vec<_>>(),
            [OTHER.parse().unwrap()]
        );
        for (kind, fields) in [
            ("rename-context-key", vec![("key", KEY), ("name", "gap")]),
            ("delete-context-key", vec![("key", KEY)]),
            ("delete-context-provision", vec![("provision", PROVISION)]),
            (
                "add-context-requirement",
                vec![("component", OTHER), ("key", KEY)],
            ),
            (
                "remove-context-requirement",
                vec![("component", OTHER), ("key", KEY)],
            ),
            ("add-slot-context-key", vec![("slot", OTHER), ("key", KEY)]),
            (
                "remove-slot-context-key",
                vec![("slot", OTHER), ("key", KEY)],
            ),
            (
                "set-context-provision-value",
                vec![("provision", PROVISION), ("value", "number 2")],
            ),
            (
                "set-context-key-default",
                vec![
                    ("key", KEY),
                    ("value_type", "number"),
                    ("default", "number 3"),
                ],
            ),
        ] {
            let mut pairs = vec![("kind", V::S(kind))];
            pairs.extend(fields.into_iter().map(|(k, v)| (k, V::S(v))));
            let input = command_input(&obj(&pairs), "commands[0]")
                .unwrap_or_else(|e| panic!("{kind}: {e}"));
            let _ = context_edit(input);
        }
        // key default가 열린 식(다른 key 읽기)이거나, 형식이 맞지 않는 식·타입·scope는 거부합니다.
        for (field, value) in [
            ("default", format_read().to_owned()),
            ("default", "length".to_owned()),
            ("default", "ast zz".to_owned()),
            ("value_type", "pixels".to_owned()),
            ("scope", "galaxy".to_owned()),
        ] {
            let mut pairs: BTreeMap<&'static str, String> = [
                ("kind", "create-context-key"),
                ("key", KEY),
                ("name", "spacing"),
                ("scope", "project"),
                ("value_type", "length"),
                ("default", "length 8px"),
            ]
            .into_iter()
            .map(|(k, v)| (k, v.to_owned()))
            .collect();
            pairs.insert(field, value.clone());
            let leaked: Vec<(&'static str, V)> = pairs
                .into_iter()
                .map(|(k, v)| (k, V::S(Box::leak(v.into_boxed_str()))))
                .collect();
            let error = command_input(&obj(&leaked), "commands[0]").unwrap_err();
            assert_eq!(
                error.code, "invalid-transaction",
                "{field}={value}: {error}"
            );
        }
    }

    fn format_view() -> &'static str {
        Box::leak(format!("view {VIEW}").into_boxed_str())
    }

    fn format_read() -> &'static str {
        Box::leak(format!("context {OTHER}").into_boxed_str())
    }

    #[test]
    fn field_kinds_parse_like_stellar_apply() {
        let visible = command_input(
            &obj(&[
                ("kind", V::S("set-visible")),
                ("node", V::S(NODE)),
                ("visible", V::B(false)),
            ]),
            "commands[0]",
        )
        .unwrap();
        assert_eq!(
            visible,
            CommandInput::SetVisible {
                node: NODE.parse().unwrap(),
                visible: false
            }
        );
        // 필드 종류에 표 밖 key, canonical 혼용, 잘못된 값 형식은 거부합니다.
        for bad in [
            obj(&[
                ("kind", V::S("set-visible")),
                ("node", V::S(NODE)),
                ("visible", V::B(true)),
                ("extra", V::B(true)),
            ]),
            obj(&[
                ("kind", V::S("set-visible")),
                ("node", V::S(NODE)),
                ("canonical", V::S("00")),
            ]),
            obj(&[
                ("kind", V::S("set-frame-width")),
                ("node", V::S(NODE)),
                ("width", V::N("-1")),
            ]),
            obj(&[
                ("kind", V::S("set-frame-width")),
                ("node", V::S(NODE)),
                ("width", V::N("1e40")),
            ]),
            obj(&[
                ("kind", V::S("move-node")),
                ("node", V::S(NODE)),
                ("new_parent", V::S(NODE)),
                ("order_key", V::S("ABC")),
            ]),
        ] {
            let error = command_input(&bad, "commands[0]").unwrap_err();
            assert_eq!(error.code, "invalid-transaction", "{error}");
        }
    }
}
