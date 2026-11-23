// 생성 TypeScript client 종단 smoke(XXI.4.1·XXI.4.3·XXI.4.4). Rust test가 node로 실행합니다.
// stdout 한 줄 = client가 보낸 요청, stdin 한 줄 = Host 응답(조각 포함). 끝나면 "RESULT <json>"을 씁니다.
import * as readline from "node:readline";
import { createHostClient, HostRpcError } from "../../bindings/stellar-host-api.ts";

let deliver: (message: string) => void = () => {};
const client = createHostClient({
  send: (message) => process.stdout.write(message + "\n"),
  onMessage: (handler) => {
    deliver = handler;
  },
});
readline.createInterface({ input: process.stdin }).on("line", (line) => deliver(line));

const text = "stellar-".repeat(2048);
const written = await client.file.write({ path: "big.txt", text });
const read = await client.file.read({ path: "big.txt" });
const batch = await client.batch([
  { method: "host.info", params: {} },
  { method: "file.list", params: {} },
  { method: "document.read_snapshot", params: {} },
]);
let denied = "none";
try {
  await client.document.readSnapshot({});
} catch (error) {
  denied = error instanceof HostRpcError ? error.code : "other";
}
const info = await client.host.info({});
const result = {
  written: written.bytes,
  same_text: read.text === text,
  batch: batch.map((item) => ("error" in item ? item.error.code : "ok")),
  denied,
  plugin_id: info.plugin_id,
};
process.stdout.write("RESULT " + JSON.stringify(result) + "\n");
process.exit(0);
