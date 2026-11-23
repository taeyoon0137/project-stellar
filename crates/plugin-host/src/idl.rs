//! Host API IDL 해석과 검사 (XXI.4.1, ADR-0097 결정 6).
//!
//! 원본은 `schemas/plugin-host-api.idl.json`입니다. [`Idl::parse`]가 형식을 엄격하게 읽고 다음을 거부합니다.
//!
//! - 중복 type·namespace·method·field·param 이름, 모르는 type 참조, 모르는 권한 이름
//! - 고빈도 namespace([`FORBIDDEN_NAMESPACES`]): pointer·render loop 호출은 Host RPC를 지나지 않습니다(XXI.4.3)
//! - 매개변수 형식이 [`ParamType`] 밖(record 매개변수 없음)
//!
//! [`conforms`]는 JSON 값이 IDL type을 따르는지 확인합니다. 결과 형태 drift test가 씁니다.

use std::collections::BTreeSet;
use std::fmt;

use crate::json::{parse, JsonLimits, JsonValue};
use crate::permission::Permission;

/// 원본 IDL 텍스트(빌드에 포함).
pub const IDL_SOURCE: &str = include_str!("../../../schemas/plugin-host-api.idl.json");

/// IDL 원본 저장소 경로.
pub const IDL_PATH: &str = "schemas/plugin-host-api.idl.json";

/// IDL에 둘 수 없는 고빈도 namespace 이름.
pub const FORBIDDEN_NAMESPACES: [&str; 5] = ["pointer", "render", "frame", "input", "animation"];

/// 매개변수 형식.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamType {
    /// 문자열.
    String,
    /// bool.
    Bool,
    /// u32 정수.
    U32,
    /// StableId 문자열.
    Id,
    /// 프로젝트 revision 문자열.
    Revision,
    /// JSON object 배열.
    JsonList,
}

impl ParamType {
    /// IDL 표기.
    pub fn idl_name(self) -> &'static str {
        match self {
            ParamType::String => "string",
            ParamType::Bool => "bool",
            ParamType::U32 => "u32",
            ParamType::Id => "id",
            ParamType::Revision => "revision",
            ParamType::JsonList => "json[]",
        }
    }

    fn from_idl(name: &str) -> Option<ParamType> {
        [
            ParamType::String,
            ParamType::Bool,
            ParamType::U32,
            ParamType::Id,
            ParamType::Revision,
            ParamType::JsonList,
        ]
        .into_iter()
        .find(|t| t.idl_name() == name)
    }
}

/// 매개변수 하나.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParamSpec {
    /// 이름.
    pub name: &'static str,
    /// 형식.
    pub ty: ParamType,
    /// 필수 여부.
    pub required: bool,
}

/// 결과 field 형식 참조.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeRef {
    /// 문자열.
    String,
    /// bool.
    Bool,
    /// 음이 아닌 정수.
    U64,
    /// StableId 문자열.
    Id,
    /// revision 문자열.
    Revision,
    /// 임의 JSON.
    Json,
    /// 이름 붙은 record.
    Record(String),
    /// 배열.
    List(Box<TypeRef>),
}

impl TypeRef {
    fn parse(text: &str) -> TypeRef {
        if let Some(inner) = text.strip_suffix("[]") {
            return TypeRef::List(Box::new(TypeRef::parse(inner)));
        }
        match text {
            "string" => TypeRef::String,
            "bool" => TypeRef::Bool,
            "u64" | "u32" => TypeRef::U64,
            "id" => TypeRef::Id,
            "revision" => TypeRef::Revision,
            "json" => TypeRef::Json,
            other => TypeRef::Record(other.to_owned()),
        }
    }
}

/// record field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldDef {
    /// 이름.
    pub name: String,
    /// 형식.
    pub ty: TypeRef,
    /// `null`일 수 있음.
    pub optional: bool,
}

/// record type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeDef {
    /// 이름(PascalCase).
    pub name: String,
    /// field(선언 순).
    pub fields: Vec<FieldDef>,
}

/// 매개변수 정의.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamDef {
    /// 이름.
    pub name: String,
    /// 형식.
    pub ty: ParamType,
    /// 필수 여부.
    pub required: bool,
}

/// method 정의.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodDef {
    /// 이름(namespace 안, snake_case).
    pub name: String,
    /// 필요 권한(`None`이면 권한 없음).
    pub permission: Option<Permission>,
    /// 매개변수.
    pub params: Vec<ParamDef>,
    /// 결과 record 이름.
    pub result: String,
    /// 설명.
    pub description: String,
}

/// namespace 정의.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamespaceDef {
    /// 이름.
    pub name: String,
    /// 설명.
    pub description: String,
    /// method.
    pub methods: Vec<MethodDef>,
}

/// 해석한 IDL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Idl {
    /// IDL 형식 버전.
    pub idl_version: u32,
    /// Host API major version.
    pub host_api_version: u32,
    /// record type.
    pub types: Vec<TypeDef>,
    /// namespace.
    pub namespaces: Vec<NamespaceDef>,
}

/// IDL 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdlError(pub String);

impl fmt::Display for IdlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for IdlError {}

fn err<T>(message: impl Into<String>) -> Result<T, IdlError> {
    Err(IdlError(message.into()))
}

fn keys(value: &JsonValue, allowed: &[&str], at: &str) -> Result<(), IdlError> {
    let pairs = value
        .as_object()
        .ok_or_else(|| IdlError(format!("{at}는 object여야 합니다")))?;
    for (key, _) in pairs {
        if !allowed.contains(&key.as_str()) {
            return err(format!("{at}에 모르는 key {key}"));
        }
    }
    Ok(())
}

fn text<'a>(value: &'a JsonValue, key: &str, at: &str) -> Result<&'a str, IdlError> {
    value
        .get(key)
        .and_then(JsonValue::as_str)
        .ok_or_else(|| IdlError(format!("{at}.{key}는 문자열이어야 합니다")))
}

fn array<'a>(value: &'a JsonValue, key: &str, at: &str) -> Result<&'a [JsonValue], IdlError> {
    value
        .get(key)
        .and_then(JsonValue::as_array)
        .ok_or_else(|| IdlError(format!("{at}.{key}는 배열이어야 합니다")))
}

fn snake(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes.next().is_some_and(|b| b.is_ascii_lowercase())
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn pascal(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes.next().is_some_and(|b| b.is_ascii_uppercase()) && bytes.all(|b| b.is_ascii_alphanumeric())
}

impl Idl {
    /// 저장소 원본 IDL.
    pub fn source() -> Idl {
        Idl::parse(IDL_SOURCE).expect("저장소 IDL은 유효해야 합니다")
    }

    /// IDL 텍스트를 해석하고 검사합니다.
    pub fn parse(source: &str) -> Result<Idl, IdlError> {
        let root = parse(source, JsonLimits::default()).map_err(|e| IdlError(e.to_string()))?;
        keys(
            &root,
            &[
                "idl_version",
                "host_api_version",
                "description",
                "types",
                "namespaces",
            ],
            "<root>",
        )?;
        let version = |key: &str| -> Result<u32, IdlError> {
            root.get(key)
                .and_then(JsonValue::as_i64)
                .and_then(|v| u32::try_from(v).ok())
                .filter(|v| *v >= 1)
                .ok_or_else(|| IdlError(format!("{key}는 1 이상의 정수여야 합니다")))
        };
        let idl_version = version("idl_version")?;
        if idl_version != 1 {
            return err("idl_version 1만 읽습니다");
        }
        let host_api_version = version("host_api_version")?;

        let mut types = Vec::new();
        let mut type_names = BTreeSet::new();
        for (i, item) in array(&root, "types", "<root>")?.iter().enumerate() {
            let at = format!("types[{i}]");
            keys(item, &["name", "fields"], &at)?;
            let name = text(item, "name", &at)?.to_owned();
            if !pascal(&name) || !type_names.insert(name.clone()) {
                return err(format!(
                    "{at}: type 이름이 PascalCase가 아니거나 중복입니다"
                ));
            }
            let mut fields = Vec::new();
            let mut field_names = BTreeSet::new();
            for (j, field) in array(item, "fields", &at)?.iter().enumerate() {
                let fat = format!("{at}.fields[{j}]");
                keys(field, &["name", "type", "optional"], &fat)?;
                let fname = text(field, "name", &fat)?.to_owned();
                if !snake(&fname) || !field_names.insert(fname.clone()) {
                    return err(format!(
                        "{fat}: field 이름이 snake_case가 아니거나 중복입니다"
                    ));
                }
                let optional = match field.get("optional") {
                    None => false,
                    Some(v) => v
                        .as_bool()
                        .ok_or_else(|| IdlError(format!("{fat}.optional은 bool")))?,
                };
                fields.push(FieldDef {
                    name: fname,
                    ty: TypeRef::parse(text(field, "type", &fat)?),
                    optional,
                });
            }
            types.push(TypeDef { name, fields });
        }

        let mut namespaces = Vec::new();
        let mut ns_names = BTreeSet::new();
        for (i, item) in array(&root, "namespaces", "<root>")?.iter().enumerate() {
            let at = format!("namespaces[{i}]");
            keys(item, &["name", "description", "methods"], &at)?;
            let name = text(item, "name", &at)?.to_owned();
            if !snake(&name) || !ns_names.insert(name.clone()) {
                return err(format!(
                    "{at}: namespace 이름이 snake_case가 아니거나 중복입니다"
                ));
            }
            if FORBIDDEN_NAMESPACES.contains(&name.as_str()) {
                return err(format!(
                    "{at}: 고빈도 namespace는 Host RPC에 둘 수 없습니다"
                ));
            }
            let description = text(item, "description", &at)?.to_owned();
            let mut methods = Vec::new();
            let mut method_names = BTreeSet::new();
            for (j, method) in array(item, "methods", &at)?.iter().enumerate() {
                let mat = format!("{at}.methods[{j}]");
                keys(
                    method,
                    &["name", "permission", "params", "result", "description"],
                    &mat,
                )?;
                let mname = text(method, "name", &mat)?.to_owned();
                if !snake(&mname) || !method_names.insert(mname.clone()) {
                    return err(format!(
                        "{mat}: method 이름이 snake_case가 아니거나 중복입니다"
                    ));
                }
                let permission = match method.get("permission") {
                    Some(JsonValue::Null) => None,
                    Some(JsonValue::String(p)) => Some(
                        Permission::from_name(p)
                            .ok_or_else(|| IdlError(format!("{mat}: 모르는 권한")))?,
                    ),
                    _ => return err(format!("{mat}.permission은 문자열 또는 null")),
                };
                let mut params = Vec::new();
                let mut param_names = BTreeSet::new();
                for (k, param) in array(method, "params", &mat)?.iter().enumerate() {
                    let pat = format!("{mat}.params[{k}]");
                    keys(param, &["name", "type", "required"], &pat)?;
                    let pname = text(param, "name", &pat)?.to_owned();
                    if !snake(&pname) || !param_names.insert(pname.clone()) {
                        return err(format!(
                            "{pat}: param 이름이 snake_case가 아니거나 중복입니다"
                        ));
                    }
                    let ty = ParamType::from_idl(text(param, "type", &pat)?)
                        .ok_or_else(|| IdlError(format!("{pat}: 매개변수 형식이 아닙니다")))?;
                    let required = param
                        .get("required")
                        .and_then(JsonValue::as_bool)
                        .ok_or_else(|| IdlError(format!("{pat}.required는 bool")))?;
                    params.push(ParamDef {
                        name: pname,
                        ty,
                        required,
                    });
                }
                let result = text(method, "result", &mat)?.to_owned();
                if !type_names.contains(&result) {
                    return err(format!("{mat}: 결과 type이 정의되지 않았습니다"));
                }
                methods.push(MethodDef {
                    name: mname,
                    permission,
                    params,
                    result,
                    description: text(method, "description", &mat)?.to_owned(),
                });
            }
            namespaces.push(NamespaceDef {
                name,
                description,
                methods,
            });
        }

        let idl = Idl {
            idl_version,
            host_api_version,
            types,
            namespaces,
        };
        for ty in &idl.types {
            for field in &ty.fields {
                idl.check_ref(&field.ty)
                    .map_err(|_| IdlError(format!("{}.{}: 모르는 type", ty.name, field.name)))?;
            }
        }
        Ok(idl)
    }

    fn check_ref(&self, ty: &TypeRef) -> Result<(), ()> {
        match ty {
            TypeRef::Record(name) => self.record(name).map(|_| ()).ok_or(()),
            TypeRef::List(inner) => self.check_ref(inner),
            _ => Ok(()),
        }
    }

    /// 이름으로 record를 찾습니다.
    pub fn record(&self, name: &str) -> Option<&TypeDef> {
        self.types.iter().find(|t| t.name == name)
    }

    /// (namespace, method) 쌍(선언 순).
    pub fn methods(&self) -> impl Iterator<Item = (&NamespaceDef, &MethodDef)> {
        self.namespaces
            .iter()
            .flat_map(|ns| ns.methods.iter().map(move |m| (ns, m)))
    }
}

/// JSON 값이 IDL type을 따르는지 확인합니다. 실패하면 위치를 돌려줍니다. record는 정의한 field만 가져야 합니다.
pub fn conforms(idl: &Idl, value: &JsonValue, ty: &TypeRef, at: &str) -> Result<(), String> {
    let ok = match ty {
        TypeRef::String | TypeRef::Id | TypeRef::Revision => value.as_str().is_some(),
        TypeRef::Bool => value.as_bool().is_some(),
        TypeRef::U64 => {
            matches!(value, JsonValue::Number(n) if n.is_integer() && !n.as_str().starts_with('-'))
        }
        TypeRef::Json => true,
        TypeRef::List(inner) => {
            let items = value
                .as_array()
                .ok_or_else(|| format!("{at}: 배열이 아님"))?;
            for (i, item) in items.iter().enumerate() {
                conforms(idl, item, inner, &format!("{at}[{i}]"))?;
            }
            true
        }
        TypeRef::Record(name) => {
            let record = idl
                .record(name)
                .ok_or_else(|| format!("{at}: 모르는 type {name}"))?;
            let pairs = value
                .as_object()
                .ok_or_else(|| format!("{at}: object가 아님"))?;
            for (key, _) in pairs {
                if !record.fields.iter().any(|f| &f.name == key) {
                    return Err(format!("{at}: {name}에 없는 field {key}"));
                }
            }
            for field in &record.fields {
                let at = format!("{at}.{}", field.name);
                match value.get(&field.name) {
                    None => return Err(format!("{at}: field 없음")),
                    Some(JsonValue::Null) if field.optional => {}
                    Some(v) => conforms(idl, v, &field.ty, &at)?,
                }
            }
            true
        }
    };
    if ok {
        Ok(())
    } else {
        Err(format!("{at}: 형식이 다름"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_idl_parses() {
        let idl = Idl::source();
        assert_eq!(idl.host_api_version, crate::HOST_API_VERSION);
        assert!(idl.methods().count() >= 10);
    }

    #[test]
    fn rejects_high_frequency_namespace_and_unknown_refs() {
        let base = IDL_SOURCE.replace("\"name\": \"network\",", "\"name\": \"pointer\",");
        assert!(Idl::parse(&base).unwrap_err().0.contains("고빈도"));
        let bad = IDL_SOURCE.replace("\"result\": \"HostInfo\"", "\"result\": \"Nope\"");
        assert!(Idl::parse(&bad).is_err());
        let perm = IDL_SOURCE.replace(
            "\"permission\": \"git.read\"",
            "\"permission\": \"git.write\"",
        );
        assert!(Idl::parse(&perm).is_err());
    }
}
