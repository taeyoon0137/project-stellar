//! IDL → Rust·TypeScript binding 생성 (XXI.4.1, ADR-0097 결정 6).
//!
//! 출력은 결정적입니다(IDL 선언 순). 생성 파일은 [`crate::generated_files`] 표로 쓰고 drift test가 비교합니다.

use std::fmt::Write as _;

use crate::idl::{Idl, ParamType, TypeRef};

/// 생성 Rust 파일 경로.
pub const RUST_PATH: &str = "crates/plugin-host/src/generated/host_api.rs";

/// 생성 TypeScript 파일 경로.
pub const TS_PATH: &str = "crates/plugin-host/bindings/stellar-host-api.ts";

fn pascal(snake: &str) -> String {
    snake
        .split('_')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(c) => c.to_ascii_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

fn camel(snake: &str) -> String {
    let p = pascal(snake);
    let mut chars = p.chars();
    match chars.next() {
        Some(c) => c.to_ascii_lowercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

fn param_variant(ty: ParamType) -> &'static str {
    match ty {
        ParamType::String => "String",
        ParamType::Bool => "Bool",
        ParamType::U32 => "U32",
        ParamType::Id => "Id",
        ParamType::Revision => "Revision",
        ParamType::JsonList => "JsonList",
    }
}

/// 생성 Rust binding.
pub fn render_rust(idl: &Idl) -> String {
    let mut out = String::new();
    out.push_str(
        "//! 생성 파일입니다. `schemas/plugin-host-api.idl.json`에서 `stellar-plugin-bindgen`이 만듭니다(ADR-0097).\n\
         //! 손으로 고치지 않습니다. IDL을 고친 뒤 `cargo run -p stellar-plugin-host --bin stellar-plugin-bindgen`을 실행합니다.\n\n\
         use crate::idl::{ParamSpec, ParamType};\nuse crate::permission::Permission;\n\n",
    );
    let _ = writeln!(
        out,
        "/// Host API major version.\npub const HOST_API_VERSION: u32 = {};\n\n/// IDL 형식 버전.\npub const IDL_VERSION: u32 = {};\n",
        idl.host_api_version, idl.idl_version
    );

    out.push_str("/// RPC namespace.\n#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]\npub enum Namespace {\n");
    for ns in &idl.namespaces {
        let _ = writeln!(out, "    /// {}\n    {},", ns.description, pascal(&ns.name));
    }
    out.push_str("}\n\nimpl Namespace {\n    /// 모든 namespace(IDL 순).\n");
    let _ = writeln!(
        out,
        "    pub const ALL: [Namespace; {}] = [{}];\n",
        idl.namespaces.len(),
        idl.namespaces
            .iter()
            .map(|ns| format!("Namespace::{}", pascal(&ns.name)))
            .collect::<Vec<_>>()
            .join(", ")
    );
    out.push_str(
        "    /// IDL 이름.\n    pub fn name(self) -> &'static str {\n        match self {\n",
    );
    for ns in &idl.namespaces {
        let _ = writeln!(
            out,
            "            Namespace::{} => \"{}\",",
            pascal(&ns.name),
            ns.name
        );
    }
    out.push_str("        }\n    }\n}\n\n");

    let methods: Vec<_> = idl.methods().collect();
    out.push_str("/// Host API method.\n#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]\npub enum Method {\n");
    for (ns, m) in &methods {
        let _ = writeln!(
            out,
            "    /// `{}.{}`: {}\n    {}{},",
            ns.name,
            m.name,
            m.description,
            pascal(&ns.name),
            pascal(&m.name)
        );
    }
    out.push_str("}\n\nimpl Method {\n    /// 모든 method(IDL 순).\n");
    let _ = writeln!(
        out,
        "    pub const ALL: [Method; {}] = [\n{}    ];\n",
        methods.len(),
        methods
            .iter()
            .map(|(ns, m)| format!("        Method::{}{},\n", pascal(&ns.name), pascal(&m.name)))
            .collect::<String>()
    );
    let arm = |out: &mut String, doc: &str, sig: &str, f: &dyn Fn(&str, &str) -> String| {
        let _ = writeln!(
            out,
            "    /// {doc}\n    pub fn {sig} {{\n        match self {{"
        );
        for (ns, m) in &methods {
            let _ = writeln!(
                out,
                "            Method::{}{} => {},",
                pascal(&ns.name),
                pascal(&m.name),
                f(&ns.name, &m.name)
            );
        }
        out.push_str("        }\n    }\n\n");
    };
    arm(
        &mut out,
        "wire 이름(`namespace.method`).",
        "name(self) -> &'static str",
        &|ns, m| format!("\"{ns}.{m}\""),
    );
    arm(
        &mut out,
        "namespace.",
        "namespace(self) -> Namespace",
        &|ns, _| format!("Namespace::{}", pascal(ns)),
    );
    let find = |ns: &str, m: &str| {
        methods
            .iter()
            .find(|(n, x)| n.name == ns && x.name == m)
            .map(|(_, x)| *x)
            .expect("method")
    };
    arm(
        &mut out,
        "필요 권한(`None`이면 권한 없음).",
        "permission(self) -> Option<Permission>",
        &|ns, m| match find(ns, m).permission {
            None => "None".to_owned(),
            Some(p) => format!("Some(Permission::{p:?})"),
        },
    );
    arm(
        &mut out,
        "결과 record 이름.",
        "result_type(self) -> &'static str",
        &|ns, m| format!("\"{}\"", find(ns, m).result),
    );
    arm(
        &mut out,
        "매개변수 표(IDL 순).",
        "params(self) -> &'static [ParamSpec]",
        &|ns, m| {
            let specs: Vec<String> = find(ns, m)
                .params
                .iter()
                .map(|p| {
                    format!(
                        "ParamSpec {{ name: \"{}\", ty: ParamType::{}, required: {} }}",
                        p.name,
                        param_variant(p.ty),
                        p.required
                    )
                })
                .collect();
            format!("&[{}]", specs.join(", "))
        },
    );
    out.push_str(
        "    /// wire 이름으로 찾습니다.\n    pub fn from_name(name: &str) -> Option<Method> {\n        Method::ALL.into_iter().find(|m| m.name() == name)\n    }\n}\n",
    );
    out
}

fn ts_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::String => "string".to_owned(),
        TypeRef::Bool => "boolean".to_owned(),
        TypeRef::U64 => "number".to_owned(),
        TypeRef::Id => "Id".to_owned(),
        TypeRef::Revision => "Revision".to_owned(),
        TypeRef::Json => "Json".to_owned(),
        TypeRef::Record(name) => name.clone(),
        TypeRef::List(inner) => format!("{}[]", ts_type(inner)),
    }
}

fn ts_param(ty: ParamType) -> &'static str {
    match ty {
        ParamType::String => "string",
        ParamType::Bool => "boolean",
        ParamType::U32 => "number",
        ParamType::Id => "Id",
        ParamType::Revision => "Revision",
        ParamType::JsonList => "Json[]",
    }
}

const TS_RUNTIME: &str = r#"
/** Host 오류. `code`는 고정 진단 code입니다. */
export interface HostError {
  code: string;
  message: string;
  data?: Json;
}

/** Host가 오류로 답한 호출. */
export class HostRpcError extends Error {
  readonly code: string;
  readonly data?: Json;
  constructor(error: HostError) {
    super(error.message);
    this.code = error.code;
    this.data = error.data;
  }
}

/** WebView bridge 같은 문자열 transport. */
export interface HostTransport {
  send(message: string): void;
  onMessage(handler: (message: string) => void): void;
}

/** batch 항목. */
export interface BatchItem<M extends MethodName = MethodName> {
  method: M;
  params: MethodMap[M]["params"];
}

/** batch 항목 결과(순서는 요청과 같음). */
export type BatchResult = { result: Json } | { error: HostError };

interface Pending {
  resolve(value: unknown): void;
  reject(error: unknown): void;
  chunks?: string[];
  received?: number;
}

/** 요청 id 부여, 응답 대응, batch, 대용량 응답 조각 재조립을 하는 client를 만듭니다. */
export function createHostClient(transport: HostTransport): HostClient {
  let next = 1;
  const pending = new Map<number, Pending>();

  const settle = (text: string): void => {
    const message = JSON.parse(text) as Record<string, unknown>;
    const id = message.id as number;
    const waiter = pending.get(id);
    if (waiter === undefined) {
      return;
    }
    const chunk = message.chunk as { index: number; count: number; data: string } | undefined;
    if (chunk !== undefined) {
      waiter.chunks ??= new Array<string>(chunk.count);
      waiter.chunks[chunk.index] = chunk.data;
      waiter.received = (waiter.received ?? 0) + 1;
      if (waiter.received === chunk.count) {
        pending.delete(id);
        settleWhole(waiter, waiter.chunks.join(""));
      }
      return;
    }
    pending.delete(id);
    settleWhole(waiter, text);
  };

  const settleWhole = (waiter: Pending, text: string): void => {
    const message = JSON.parse(text) as Record<string, unknown>;
    if (message.error !== undefined) {
      waiter.reject(new HostRpcError(message.error as HostError));
    } else if (message.batch !== undefined) {
      waiter.resolve(message.batch);
    } else {
      waiter.resolve(message.result);
    }
  };

  transport.onMessage(settle);

  const send = (body: Record<string, unknown>): Promise<unknown> =>
    new Promise((resolve, reject) => {
      const id = next++;
      pending.set(id, { resolve, reject });
      transport.send(JSON.stringify({ stellar_rpc: STELLAR_RPC_VERSION, id, ...body }));
    });

  const call = <M extends MethodName>(method: M, params: MethodMap[M]["params"]) =>
    send({ method, params }) as Promise<MethodMap[M]["result"]>;
  const batch = (items: BatchItem[]) => send({ batch: items }) as Promise<BatchResult[]>;
  return bindNamespaces(call, batch);
}
"#;

/// 생성 TypeScript binding.
pub fn render_ts(idl: &Idl) -> String {
    let mut out = String::new();
    out.push_str(
        "// 생성 파일입니다. schemas/plugin-host-api.idl.json에서 stellar-plugin-bindgen이 만듭니다(ADR-0097).\n\
         // 손으로 고치지 않습니다.\n\n",
    );
    let _ = writeln!(
        out,
        "/** Host API major version. */\nexport const HOST_API_VERSION = {} as const;\n\n/** RPC envelope version. */\nexport const STELLAR_RPC_VERSION = 1 as const;\n\n/** StableId 문자열. */\nexport type Id = string;\n/** 프로젝트 revision 문자열. */\nexport type Revision = string;\n/** 임의 JSON 값. */\nexport type Json = unknown;\n",
        idl.host_api_version
    );
    for ty in &idl.types {
        let _ = writeln!(out, "export interface {} {{", ty.name);
        for f in &ty.fields {
            let suffix = if f.optional { " | null" } else { "" };
            let _ = writeln!(out, "  {}: {}{};", f.name, ts_type(&f.ty), suffix);
        }
        out.push_str("}\n\n");
    }
    for (ns, m) in idl.methods() {
        let _ = writeln!(
            out,
            "export interface {}{}Params {{",
            pascal(&ns.name),
            pascal(&m.name)
        );
        for p in &m.params {
            let opt = if p.required { "" } else { "?" };
            let _ = writeln!(out, "  {}{}: {};", p.name, opt, ts_param(p.ty));
        }
        out.push_str("}\n\n");
    }
    out.push_str("/** method 이름 → 매개변수·결과 형식. */\nexport interface MethodMap {\n");
    for (ns, m) in idl.methods() {
        let _ = writeln!(
            out,
            "  \"{}.{}\": {{ params: {}{}Params; result: {} }};",
            ns.name,
            m.name,
            pascal(&ns.name),
            pascal(&m.name),
            m.result
        );
    }
    out.push_str("}\n\n/** method 이름. */\nexport type MethodName = keyof MethodMap;\n\n");
    out.push_str("/** 모든 method(IDL 순). */\nexport const METHODS: readonly MethodName[] = [\n");
    for (ns, m) in idl.methods() {
        let _ = writeln!(out, "  \"{}.{}\",", ns.name, m.name);
    }
    out.push_str("];\n\n/** namespace별 호출. */\nexport interface HostClient {\n");
    out.push_str("  call<M extends MethodName>(method: M, params: MethodMap[M][\"params\"]): Promise<MethodMap[M][\"result\"]>;\n");
    out.push_str("  batch(items: BatchItem[]): Promise<BatchResult[]>;\n");
    for ns in &idl.namespaces {
        let _ = writeln!(out, "  {}: {{", camel(&ns.name));
        for m in &ns.methods {
            let _ = writeln!(
                out,
                "    {}(params: {}{}Params): Promise<{}>;",
                camel(&m.name),
                pascal(&ns.name),
                pascal(&m.name),
                m.result
            );
        }
        out.push_str("  };\n");
    }
    out.push_str("}\n");
    out.push_str(TS_RUNTIME);
    out.push_str("\nfunction bindNamespaces(\n  call: <M extends MethodName>(method: M, params: MethodMap[M][\"params\"]) => Promise<MethodMap[M][\"result\"]>,\n  batch: (items: BatchItem[]) => Promise<BatchResult[]>,\n): HostClient {\n  return {\n    call,\n    batch,\n");
    for ns in &idl.namespaces {
        let _ = writeln!(out, "    {}: {{", camel(&ns.name));
        for m in &ns.methods {
            let _ = writeln!(
                out,
                "      {}: (params) => call(\"{}.{}\", params),",
                camel(&m.name),
                ns.name,
                m.name
            );
        }
        out.push_str("    },\n");
    }
    out.push_str("  };\n}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_conversion() {
        assert_eq!(pascal("read_snapshot"), "ReadSnapshot");
        assert_eq!(camel("tree_page"), "treePage");
    }
}
