// 생성 파일입니다. schemas/plugin-host-api.idl.json에서 stellar-plugin-bindgen이 만듭니다(ADR-0097).
// 손으로 고치지 않습니다.

/** Host API major version. */
export const HOST_API_VERSION = 1 as const;

/** RPC envelope version. */
export const STELLAR_RPC_VERSION = 1 as const;

/** StableId 문자열. */
export type Id = string;
/** 프로젝트 revision 문자열. */
export type Revision = string;
/** 임의 JSON 값. */
export type Json = unknown;

export interface RpcLimits {
  max_message_bytes: number;
  max_batch_items: number;
  max_in_flight: number;
  max_messages_per_second: number;
  chunk_bytes: number;
}

export interface HostInfo {
  host_api_version: number;
  plugin_id: string;
  plugin_version: string;
  permissions: string[];
  limits: RpcLimits;
}

export interface ViewOutline {
  id: Id;
  display_name: string;
  root_frame: Id;
}

export interface PageOutline {
  id: Id;
  display_name: string;
  views: ViewOutline[];
}

export interface DocumentOutline {
  id: Id;
  display_name: string;
  pages: PageOutline[];
}

export interface ProjectSnapshot {
  revision: Revision;
  documents: DocumentOutline[];
}

export interface TreeNode {
  id: Id;
  parent: Id | null;
  view: Id;
  depth: number;
  kind: string;
  display_name: string;
  visible: boolean;
  locked: boolean;
  child_count: number;
}

export interface TreePage {
  revision: Revision;
  document: Id;
  total: number;
  offset: number;
  nodes: TreeNode[];
  next_cursor: string | null;
}

export interface PropertyValue {
  type: string;
  value: Json;
}

export interface NodeProperty {
  path: string;
  key: string;
  value: PropertyValue;
}

export interface NodeDetail {
  revision: Revision;
  document: Id;
  id: Id;
  parent: Id | null;
  kind: string;
  display_name: string;
  visible: boolean;
  locked: boolean;
  children: Id[];
  properties: NodeProperty[];
  modifier_count: number;
}

export interface DefinitionSet {
  revision: Revision;
  chunk_kind: string;
  chunk_count: number;
}

export interface DiffPosition {
  parent: Id | null;
  order_key_hex: string | null;
}

export interface DiffEntry {
  kind: string;
  document: Id | null;
  component: Id | null;
  object: Id;
  object_kind: string | null;
  path: string | null;
  key: string | null;
  old: PropertyValue | null;
  new: PropertyValue | null;
  from: DiffPosition | null;
  to: DiffPosition | null;
}

export interface ApplyResult {
  mode: string;
  base_revision: Revision;
  revision: Revision;
  document: Id;
  command_count: number;
  written_chunks: number;
  change_count: number;
  changes: DiffEntry[];
  truncated: boolean;
}

export interface GitHead {
  state: string;
  branch: string | null;
  commit: string | null;
}

export interface GitChange {
  id: string;
  area: string;
  kind: string;
  object: Id;
  document: Id | null;
  property: string | null;
}

export interface GitSemanticPage {
  staged_total: number;
  unstaged_total: number;
  changes: GitChange[];
  next_cursor: string | null;
}

export interface GitStatus {
  git_revision: string;
  head: GitHead;
  semantic: GitSemanticPage | null;
}

export interface FileContent {
  path: string;
  text: string;
}

export interface FileWritten {
  path: string;
  bytes: number;
}

export interface FileList {
  paths: string[];
}

export interface NetworkResponse {
  status: number;
  text: string;
}

export interface HostInfoParams {
}

export interface DocumentReadSnapshotParams {
}

export interface QueryTreePageParams {
  document: Id;
  cursor?: string;
  limit?: number;
}

export interface QueryNodeParams {
  document: Id;
  node: Id;
}

export interface QueryTokensParams {
}

export interface QueryComponentsParams {
}

export interface CommandApplyParams {
  base_revision: Revision;
  document: Id;
  commands: Json[];
  dry_run?: boolean;
}

export interface GitStatusParams {
  cursor?: string;
  limit?: number;
}

export interface FileReadParams {
  path: string;
}

export interface FileWriteParams {
  path: string;
  text: string;
}

export interface FileListParams {
}

export interface NetworkFetchParams {
  url: string;
}

/** method 이름 → 매개변수·결과 형식. */
export interface MethodMap {
  "host.info": { params: HostInfoParams; result: HostInfo };
  "document.read_snapshot": { params: DocumentReadSnapshotParams; result: ProjectSnapshot };
  "query.tree_page": { params: QueryTreePageParams; result: TreePage };
  "query.node": { params: QueryNodeParams; result: NodeDetail };
  "query.tokens": { params: QueryTokensParams; result: DefinitionSet };
  "query.components": { params: QueryComponentsParams; result: DefinitionSet };
  "command.apply": { params: CommandApplyParams; result: ApplyResult };
  "git.status": { params: GitStatusParams; result: GitStatus };
  "file.read": { params: FileReadParams; result: FileContent };
  "file.write": { params: FileWriteParams; result: FileWritten };
  "file.list": { params: FileListParams; result: FileList };
  "network.fetch": { params: NetworkFetchParams; result: NetworkResponse };
}

/** method 이름. */
export type MethodName = keyof MethodMap;

/** 모든 method(IDL 순). */
export const METHODS: readonly MethodName[] = [
  "host.info",
  "document.read_snapshot",
  "query.tree_page",
  "query.node",
  "query.tokens",
  "query.components",
  "command.apply",
  "git.status",
  "file.read",
  "file.write",
  "file.list",
  "network.fetch",
];

/** namespace별 호출. */
export interface HostClient {
  call<M extends MethodName>(method: M, params: MethodMap[M]["params"]): Promise<MethodMap[M]["result"]>;
  batch(items: BatchItem[]): Promise<BatchResult[]>;
  host: {
    info(params: HostInfoParams): Promise<HostInfo>;
  };
  document: {
    readSnapshot(params: DocumentReadSnapshotParams): Promise<ProjectSnapshot>;
  };
  query: {
    treePage(params: QueryTreePageParams): Promise<TreePage>;
    node(params: QueryNodeParams): Promise<NodeDetail>;
    tokens(params: QueryTokensParams): Promise<DefinitionSet>;
    components(params: QueryComponentsParams): Promise<DefinitionSet>;
  };
  command: {
    apply(params: CommandApplyParams): Promise<ApplyResult>;
  };
  git: {
    status(params: GitStatusParams): Promise<GitStatus>;
  };
  file: {
    read(params: FileReadParams): Promise<FileContent>;
    write(params: FileWriteParams): Promise<FileWritten>;
    list(params: FileListParams): Promise<FileList>;
  };
  network: {
    fetch(params: NetworkFetchParams): Promise<NetworkResponse>;
  };
}

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

function bindNamespaces(
  call: <M extends MethodName>(method: M, params: MethodMap[M]["params"]) => Promise<MethodMap[M]["result"]>,
  batch: (items: BatchItem[]) => Promise<BatchResult[]>,
): HostClient {
  return {
    call,
    batch,
    host: {
      info: (params) => call("host.info", params),
    },
    document: {
      readSnapshot: (params) => call("document.read_snapshot", params),
    },
    query: {
      treePage: (params) => call("query.tree_page", params),
      node: (params) => call("query.node", params),
      tokens: (params) => call("query.tokens", params),
      components: (params) => call("query.components", params),
    },
    command: {
      apply: (params) => call("command.apply", params),
    },
    git: {
      status: (params) => call("git.status", params),
    },
    file: {
      read: (params) => call("file.read", params),
      write: (params) => call("file.write", params),
      list: (params) => call("file.list", params),
    },
    network: {
      fetch: (params) => call("network.fetch", params),
    },
  };
}
